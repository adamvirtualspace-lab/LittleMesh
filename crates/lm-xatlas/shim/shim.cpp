// Flat C API over xatlas, so the Rust side can declare it by hand without
// bindgen. Every struct here is mirrored in src/ffi.rs; keep them in sync.

#include <stdint.h>
#include <string.h>

#include "xatlas.h"

extern "C" {

struct LmMeshInput {
	const float *positions; // xyz, vertex_count * 3
	const float *normals; // optional, xyz
	const float *uvs; // optional, uv
	const uint32_t *indices; // index_count
	const uint32_t *face_materials; // optional, index_count / 3
	uint32_t vertex_count;
	uint32_t index_count;
};

struct LmUvInput {
	const float *uvs; // uv, vertex_count * 2
	const uint32_t *indices;
	const uint32_t *face_materials; // optional
	uint32_t vertex_count;
	uint32_t index_count;
};

struct LmChartOptions {
	float max_chart_area;
	float max_boundary_length;
	float normal_deviation_weight;
	float roundness_weight;
	float straightness_weight;
	float normal_seam_weight;
	float texture_seam_weight;
	float max_cost;
	uint32_t max_iterations;
	uint32_t use_input_mesh_uvs;
	uint32_t fix_winding;
};

struct LmPackOptions {
	uint32_t max_chart_size;
	uint32_t padding;
	float texels_per_unit;
	uint32_t resolution;
	uint32_t bilinear;
	uint32_t block_align;
	uint32_t brute_force;
	uint32_t rotate_charts_to_axis;
	uint32_t rotate_charts;
	uint32_t keep_chart_aspect;
};

struct LmAtlasInfo {
	uint32_t width;
	uint32_t height;
	uint32_t atlas_count;
	uint32_t chart_count;
	uint32_t mesh_count;
	float texels_per_unit;
};

struct LmMeshInfo {
	uint32_t vertex_count;
	uint32_t index_count;
	uint32_t chart_count;
};

void *lm_xatlas_create(void) {
	xatlas::SetPrint(nullptr, false);
	return xatlas::Create();
}

void lm_xatlas_destroy(void *atlas) {
	xatlas::Destroy(static_cast<xatlas::Atlas *>(atlas));
}

int32_t lm_xatlas_add_mesh(void *atlas, const LmMeshInput *in) {
	xatlas::MeshDecl decl;
	decl.vertexPositionData = in->positions;
	decl.vertexPositionStride = sizeof(float) * 3;
	if (in->normals) {
		decl.vertexNormalData = in->normals;
		decl.vertexNormalStride = sizeof(float) * 3;
	}
	if (in->uvs) {
		decl.vertexUvData = in->uvs;
		decl.vertexUvStride = sizeof(float) * 2;
	}
	decl.indexData = in->indices;
	decl.indexFormat = xatlas::IndexFormat::UInt32;
	decl.faceMaterialData = in->face_materials;
	decl.vertexCount = in->vertex_count;
	decl.indexCount = in->index_count;
	return static_cast<int32_t>(xatlas::AddMesh(static_cast<xatlas::Atlas *>(atlas), decl));
}

int32_t lm_xatlas_add_uv_mesh(void *atlas, const LmUvInput *in) {
	xatlas::UvMeshDecl decl;
	decl.vertexUvData = in->uvs;
	decl.vertexStride = sizeof(float) * 2;
	decl.indexData = in->indices;
	decl.indexFormat = xatlas::IndexFormat::UInt32;
	decl.faceMaterialData = in->face_materials;
	decl.vertexCount = in->vertex_count;
	decl.indexCount = in->index_count;
	return static_cast<int32_t>(xatlas::AddUvMesh(static_cast<xatlas::Atlas *>(atlas), decl));
}

void lm_xatlas_compute_charts(void *atlas, const LmChartOptions *o) {
	xatlas::ChartOptions c;
	c.maxChartArea = o->max_chart_area;
	c.maxBoundaryLength = o->max_boundary_length;
	c.normalDeviationWeight = o->normal_deviation_weight;
	c.roundnessWeight = o->roundness_weight;
	c.straightnessWeight = o->straightness_weight;
	c.normalSeamWeight = o->normal_seam_weight;
	c.textureSeamWeight = o->texture_seam_weight;
	c.maxCost = o->max_cost;
	c.maxIterations = o->max_iterations;
	c.useInputMeshUvs = o->use_input_mesh_uvs != 0;
	c.fixWinding = o->fix_winding != 0;
	xatlas::ComputeCharts(static_cast<xatlas::Atlas *>(atlas), c);
}

void lm_xatlas_pack_charts(void *atlas, const LmPackOptions *o) {
	xatlas::PackOptions p;
	p.maxChartSize = o->max_chart_size;
	p.padding = o->padding;
	p.texelsPerUnit = o->texels_per_unit;
	p.resolution = o->resolution;
	p.bilinear = o->bilinear != 0;
	p.blockAlign = o->block_align != 0;
	p.bruteForce = o->brute_force != 0;
	p.createImage = false;
	p.rotateChartsToAxis = o->rotate_charts_to_axis != 0;
	p.rotateCharts = o->rotate_charts != 0;
	p.keepChartAspect = o->keep_chart_aspect != 0;
	xatlas::PackCharts(static_cast<xatlas::Atlas *>(atlas), p);
}

void lm_xatlas_info(const void *atlas, LmAtlasInfo *out) {
	const xatlas::Atlas *a = static_cast<const xatlas::Atlas *>(atlas);
	out->width = a->width;
	out->height = a->height;
	out->atlas_count = a->atlasCount;
	out->chart_count = a->chartCount;
	out->mesh_count = a->meshCount;
	out->texels_per_unit = a->texelsPerUnit;
}

// Returns 0 on success, -1 if `mesh` is out of range or nothing was packed yet.
int32_t lm_xatlas_mesh_info(const void *atlas, uint32_t mesh, LmMeshInfo *out) {
	const xatlas::Atlas *a = static_cast<const xatlas::Atlas *>(atlas);
	if (!a->meshes || mesh >= a->meshCount)
		return -1;
	const xatlas::Mesh &m = a->meshes[mesh];
	out->vertex_count = m.vertexCount;
	out->index_count = m.indexCount;
	out->chart_count = m.chartCount;
	return 0;
}

// Copies one output mesh into caller-allocated arrays sized from lm_xatlas_mesh_info:
// xrefs and chart/atlas indices per vertex, uvs as 2 floats per vertex (in texels).
int32_t lm_xatlas_mesh_copy(const void *atlas, uint32_t mesh, uint32_t *xrefs, float *uvs, int32_t *charts, int32_t *atlases, uint32_t *indices) {
	const xatlas::Atlas *a = static_cast<const xatlas::Atlas *>(atlas);
	if (!a->meshes || mesh >= a->meshCount)
		return -1;
	const xatlas::Mesh &m = a->meshes[mesh];
	for (uint32_t i = 0; i < m.vertexCount; i++) {
		const xatlas::Vertex &v = m.vertexArray[i];
		xrefs[i] = v.xref;
		uvs[i * 2] = v.uv[0];
		uvs[i * 2 + 1] = v.uv[1];
		charts[i] = v.chartIndex;
		atlases[i] = v.atlasIndex;
	}
	memcpy(indices, m.indexArray, sizeof(uint32_t) * m.indexCount);
	return 0;
}

} // extern "C"
