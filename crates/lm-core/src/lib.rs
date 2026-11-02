//! LittleMesh core: mesh types, topology, import/export, and processing stages.

pub mod halfedge;
pub mod io;
pub mod mesh;
pub mod primitives;
pub mod stats;

pub use halfedge::HalfEdgeMesh;
pub use mesh::{AlphaMode, Image, Material, Mesh, Scene, Submesh, TextureRef};

/// Errors produced by the core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("glTF error: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("OBJ error: {0}")]
    Obj(#[from] tobj::LoadError),
    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),
    #[error("invalid mesh '{mesh}': {reason}")]
    InvalidMesh { mesh: String, reason: String },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
