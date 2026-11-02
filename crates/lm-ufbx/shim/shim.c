// Flattens a ufbx scene into what LittleMesh needs: one triangle mesh per
// node instance in world space (Y up, metres), split into parts by material,
// plus materials and the unique texture files they use. Declarations for Rust
// are in src/ffi.rs; the layouts must match.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ufbx.h"

typedef struct LmStr {
    const char *data;
    size_t len;
} LmStr;

typedef struct LmFbxPart {
    uint32_t index_start;
    uint32_t index_count;
    int32_t material; // index into the materials, or -1
} LmFbxPart;

typedef struct LmFbxMesh {
    LmStr name;
    uint32_t vertex_count;
    uint32_t index_count;
    uint32_t uv_set_count;
    uint32_t has_colors;
    uint32_t part_count;
    const float *positions; // 3 per vertex
    const float *normals;   // 3 per vertex
    const float *uvs;       // 2 per vertex, one block of vertex_count per set
    const float *colors;    // 4 per vertex, or NULL
    const uint32_t *indices;
    const LmFbxPart *parts;
} LmFbxMesh;

typedef struct LmFbxMaterial {
    LmStr name;
    float base_color[4];
    float metallic;
    float roughness;
    float emissive[3];
    // Indices into the texture files, or -1.
    int32_t base_color_texture;
    int32_t normal_texture;
    int32_t occlusion_texture;
    int32_t emissive_texture;
    uint32_t double_sided;
} LmFbxMaterial;

typedef struct LmFbxTexture {
    LmStr filename;          // resolved against the loaded file
    LmStr relative_filename; // as written in the file
    LmStr absolute_filename; // as written in the file
    const uint8_t *content;  // embedded bytes, or NULL
    size_t content_size;
} LmFbxTexture;

typedef struct LmFbxScene {
    ufbx_scene *scene;
    LmFbxMesh *meshes;
    size_t mesh_count;
    LmFbxMaterial *materials;
    size_t material_count;
    LmFbxTexture *textures;
    size_t texture_count;
    // Faces with fewer than three corners that were dropped.
    size_t skipped_faces;
} LmFbxScene;

static LmStr lm_str(ufbx_string s) {
    LmStr out = {s.data, s.length};
    return out;
}

static int32_t lm_texture_index(const ufbx_material_map *map) {
    const ufbx_texture *t = map->texture;
    if (!t || !map->texture_enabled) {
        return -1;
    }
    // Layered and shader textures: use the first file underneath.
    if (!t->has_file && t->file_textures.count > 0) {
        t = t->file_textures.data[0];
    }
    return t->has_file ? (int32_t)t->file_index : -1;
}

static float lm_real(const ufbx_material_map *map, float fallback) {
    return map->has_value ? (float)map->value_real : fallback;
}

static void lm_convert_material(const ufbx_material *m, LmFbxMaterial *out) {
    const ufbx_material_pbr_maps *pbr = &m->pbr;
    float factor = lm_real(&pbr->base_factor, 1.0f);
    ufbx_vec4 color = pbr->base_color.has_value ? pbr->base_color.value_vec4 : (ufbx_vec4){1, 1, 1, 1};
    out->base_color_texture = lm_texture_index(&pbr->base_color);
    // In FBX a texture replaces the colour; glTF multiplies them.
    if (out->base_color_texture >= 0) {
        color = (ufbx_vec4){1, 1, 1, 1};
    }
    out->name = lm_str(m->name);
    out->base_color[0] = (float)color.x * factor;
    out->base_color[1] = (float)color.y * factor;
    out->base_color[2] = (float)color.z * factor;
    out->base_color[3] = lm_real(&pbr->opacity, 1.0f);
    out->metallic = lm_real(&pbr->metalness, 0.0f);
    out->roughness = lm_real(&pbr->roughness, 1.0f);
    float emission = lm_real(&pbr->emission_factor, 1.0f);
    ufbx_vec3 e = pbr->emission_color.has_value ? pbr->emission_color.value_vec3 : (ufbx_vec3){0, 0, 0};
    out->emissive[0] = (float)e.x * emission;
    out->emissive[1] = (float)e.y * emission;
    out->emissive[2] = (float)e.z * emission;
    out->normal_texture = lm_texture_index(&pbr->normal_map);
    if (out->normal_texture < 0) {
        out->normal_texture = lm_texture_index(&m->fbx.normal_map);
    }
    out->occlusion_texture = lm_texture_index(&pbr->ambient_occlusion);
    out->emissive_texture = lm_texture_index(&pbr->emission_color);
    out->double_sided = m->features.double_sided.enabled;
}

// Corner attributes before deduplication; memcmp'd by ufbx_generate_indices,
// so every byte is written.
typedef struct LmStreams {
    float *positions, *normals, *uvs, *colors;
} LmStreams;

static void lm_free_mesh(LmFbxMesh *m) {
    free((void *)m->positions);
    free((void *)m->normals);
    free((void *)m->uvs);
    free((void *)m->colors);
    free((void *)m->indices);
    free((void *)m->parts);
}

// Returns 0 on success, -1 on allocation or ufbx failure.
static int lm_convert_mesh(LmFbxScene *out, const ufbx_node *node, LmFbxMesh *dst) {
    const ufbx_mesh *mesh = node->mesh;
    const ufbx_matrix *world = &node->geometry_to_world;
    ufbx_matrix normal_matrix = ufbx_matrix_for_normals(world);
    int mirrored = ufbx_matrix_determinant(world) < 0.0;
    size_t uv_sets = mesh->uv_sets.count;
    int colors = mesh->vertex_color.exists;

    memset(dst, 0, sizeof(*dst));
    dst->name = lm_str(node->name.length > 0 ? node->name : mesh->name);

    size_t corners = 0;
    for (size_t p = 0; p < mesh->material_parts.count; p++) {
        corners += mesh->material_parts.data[p].num_triangles * 3;
    }
    if (corners == 0 || corners > UINT32_MAX) {
        return corners == 0 ? 0 : -1;
    }

    LmStreams s = {0};
    size_t tri_cap = mesh->max_face_triangles * 3;
    uint32_t *tri = malloc(tri_cap * sizeof(uint32_t));
    uint32_t *indices = malloc(corners * sizeof(uint32_t));
    LmFbxPart *parts = calloc(mesh->material_parts.count, sizeof(LmFbxPart));
    s.positions = malloc(corners * 3 * sizeof(float));
    s.normals = malloc(corners * 3 * sizeof(float));
    // One interleaved stream of all UV sets per corner while deduplicating.
    s.uvs = uv_sets ? malloc(corners * uv_sets * 2 * sizeof(float)) : NULL;
    s.colors = colors ? malloc(corners * 4 * sizeof(float)) : NULL;
    if (!tri || !indices || !parts || !s.positions || !s.normals || (uv_sets && !s.uvs) || (colors && !s.colors)) {
        goto fail;
    }

    size_t n = 0;
    uint32_t part_count = 0;
    for (size_t p = 0; p < mesh->material_parts.count; p++) {
        const ufbx_mesh_part *part = &mesh->material_parts.data[p];
        if (part->num_triangles == 0) {
            continue;
        }
        size_t start = n;
        for (size_t f = 0; f < part->num_faces; f++) {
            ufbx_face face = mesh->faces.data[part->face_indices.data[f]];
            if (face.num_indices < 3) {
                out->skipped_faces++;
                continue;
            }
            uint32_t count = ufbx_triangulate_face(tri, tri_cap, mesh, face);
            for (uint32_t t = 0; t < count; t++) {
                for (int k = 0; k < 3; k++) {
                    // Mirroring transforms flip the winding back.
                    int corner = mirrored ? (3 - k) % 3 : k;
                    uint32_t ix = tri[t * 3 + corner];
                    ufbx_vec3 pos = ufbx_transform_position(world, ufbx_get_vertex_vec3(&mesh->vertex_position, ix));
                    ufbx_vec3 nrm = ufbx_transform_direction(&normal_matrix, ufbx_get_vertex_vec3(&mesh->vertex_normal, ix));
                    nrm = ufbx_vec3_normalize(nrm);
                    s.positions[n * 3 + 0] = (float)pos.x;
                    s.positions[n * 3 + 1] = (float)pos.y;
                    s.positions[n * 3 + 2] = (float)pos.z;
                    s.normals[n * 3 + 0] = (float)nrm.x;
                    s.normals[n * 3 + 1] = (float)nrm.y;
                    s.normals[n * 3 + 2] = (float)nrm.z;
                    for (size_t u = 0; u < uv_sets; u++) {
                        const ufbx_vertex_vec2 *uv = &mesh->uv_sets.data[u].vertex_uv;
                        ufbx_vec2 v = uv->exists ? ufbx_get_vertex_vec2(uv, ix) : (ufbx_vec2){0, 0};
                        s.uvs[(n * uv_sets + u) * 2 + 0] = (float)v.x;
                        s.uvs[(n * uv_sets + u) * 2 + 1] = (float)v.y;
                    }
                    if (colors) {
                        ufbx_vec4 c = ufbx_get_vertex_vec4(&mesh->vertex_color, ix);
                        s.colors[n * 4 + 0] = (float)c.x;
                        s.colors[n * 4 + 1] = (float)c.y;
                        s.colors[n * 4 + 2] = (float)c.z;
                        s.colors[n * 4 + 3] = (float)c.w;
                    }
                    n++;
                }
            }
        }
        if (n == start) {
            continue;
        }
        // Per-instance materials win over the mesh's own.
        const ufbx_material *material = NULL;
        if (p < node->materials.count) {
            material = node->materials.data[p];
        } else if (p < mesh->materials.count) {
            material = mesh->materials.data[p];
        }
        parts[part_count].index_start = (uint32_t)start;
        parts[part_count].index_count = (uint32_t)(n - start);
        parts[part_count].material = material ? (int32_t)material->typed_id : -1;
        part_count++;
    }
    if (n == 0) {
        free(tri);
        free(indices);
        free(parts);
        free(s.positions);
        free(s.normals);
        free(s.uvs);
        free(s.colors);
        return 0;
    }

    ufbx_vertex_stream streams[4];
    size_t stream_count = 0;
    streams[stream_count++] = (ufbx_vertex_stream){s.positions, n, 3 * sizeof(float)};
    streams[stream_count++] = (ufbx_vertex_stream){s.normals, n, 3 * sizeof(float)};
    if (uv_sets) {
        streams[stream_count++] = (ufbx_vertex_stream){s.uvs, n, uv_sets * 2 * sizeof(float)};
    }
    if (colors) {
        streams[stream_count++] = (ufbx_vertex_stream){s.colors, n, 4 * sizeof(float)};
    }
    ufbx_error error;
    size_t vertices = ufbx_generate_indices(streams, stream_count, indices, n, NULL, &error);
    if (error.type != UFBX_ERROR_NONE) {
        goto fail;
    }

    // Split the interleaved UV sets into one block per set.
    if (uv_sets > 1) {
        float *split = malloc(vertices * uv_sets * 2 * sizeof(float));
        if (!split) {
            goto fail;
        }
        for (size_t v = 0; v < vertices; v++) {
            for (size_t u = 0; u < uv_sets; u++) {
                split[(u * vertices + v) * 2 + 0] = s.uvs[(v * uv_sets + u) * 2 + 0];
                split[(u * vertices + v) * 2 + 1] = s.uvs[(v * uv_sets + u) * 2 + 1];
            }
        }
        free(s.uvs);
        s.uvs = split;
    }

    free(tri);
    dst->vertex_count = (uint32_t)vertices;
    dst->index_count = (uint32_t)n;
    dst->uv_set_count = (uint32_t)uv_sets;
    dst->has_colors = (uint32_t)colors;
    dst->part_count = part_count;
    dst->positions = s.positions;
    dst->normals = s.normals;
    dst->uvs = s.uvs;
    dst->colors = s.colors;
    dst->indices = indices;
    dst->parts = parts;
    return 0;

fail:
    free(tri);
    free(indices);
    free(parts);
    free(s.positions);
    free(s.normals);
    free(s.uvs);
    free(s.colors);
    return -1;
}

void lm_fbx_free(LmFbxScene *s) {
    if (!s) {
        return;
    }
    for (size_t i = 0; i < s->mesh_count; i++) {
        lm_free_mesh(&s->meshes[i]);
    }
    free(s->meshes);
    free(s->materials);
    free(s->textures);
    ufbx_free_scene(s->scene);
    free(s);
}

// Loads `path` (UTF-8, `len` bytes). On failure returns NULL and writes a
// NUL-terminated message to `error`.
LmFbxScene *lm_fbx_load(const char *path, size_t len, char *error, size_t error_size) {
    ufbx_load_opts opts = {0};
    opts.target_axes = ufbx_axes_right_handed_y_up;
    opts.target_unit_meters = 1.0f;
    opts.space_conversion = UFBX_SPACE_CONVERSION_MODIFY_GEOMETRY;
    opts.generate_missing_normals = true;
    opts.ignore_animation = true;

    ufbx_error err;
    ufbx_scene *scene = ufbx_load_file_len(path, len, &opts, &err);
    if (!scene) {
        ufbx_format_error(error, error_size, &err);
        return NULL;
    }
    LmFbxScene *s = calloc(1, sizeof(LmFbxScene));
    if (!s) {
        ufbx_free_scene(scene);
        snprintf(error, error_size, "out of memory");
        return NULL;
    }
    s->scene = scene;

    s->material_count = scene->materials.count;
    s->materials = calloc(s->material_count ? s->material_count : 1, sizeof(LmFbxMaterial));
    s->texture_count = scene->texture_files.count;
    s->textures = calloc(s->texture_count ? s->texture_count : 1, sizeof(LmFbxTexture));
    s->meshes = calloc(scene->nodes.count ? scene->nodes.count : 1, sizeof(LmFbxMesh));
    if (!s->materials || !s->textures || !s->meshes) {
        goto oom;
    }
    for (size_t i = 0; i < s->material_count; i++) {
        lm_convert_material(scene->materials.data[i], &s->materials[i]);
    }
    for (size_t i = 0; i < s->texture_count; i++) {
        const ufbx_texture_file *f = &scene->texture_files.data[i];
        LmFbxTexture *t = &s->textures[i];
        t->filename = lm_str(f->filename);
        t->relative_filename = lm_str(f->relative_filename);
        t->absolute_filename = lm_str(f->absolute_filename);
        t->content = f->content.size ? (const uint8_t *)f->content.data : NULL;
        t->content_size = f->content.size;
    }
    for (size_t i = 0; i < scene->nodes.count; i++) {
        const ufbx_node *node = scene->nodes.data[i];
        if (!node->mesh) {
            continue;
        }
        if (lm_convert_mesh(s, node, &s->meshes[s->mesh_count]) != 0) {
            goto oom;
        }
        if (s->meshes[s->mesh_count].index_count > 0) {
            s->mesh_count++;
        }
    }
    return s;

oom:
    lm_fbx_free(s);
    snprintf(error, error_size, "out of memory while converting the scene");
    return NULL;
}
