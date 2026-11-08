//! Programmatic fixtures for intake tests.
//!
//! All fixtures are generated in-memory and written into a temp directory
//! per test. No binary fixtures are committed: the GLB is assembled from
//! its JSON/BIN chunks here, which doubles as documentation of the minimal
//! container layout the scanner must accept.

use std::fs;
use std::path::{Path, PathBuf};

/// A valid 1x1 grayscale PNG (standard minimal image), decoded from its
/// base64 form using the crate's own decoder (which is unit-tested against
/// the RFC 4648 vectors).
pub fn png_bytes() -> Vec<u8> {
    const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    michelangelo_asset_intake::base64::decode(B64).expect("fixture PNG base64 must decode")
}

/// One triangle: 3 positions (f32) + 3 u16 indices.
pub fn triangle_bin() -> Vec<u8> {
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        bin.extend_from_slice(&v[0].to_le_bytes());
        bin.extend_from_slice(&v[1].to_le_bytes());
        bin.extend_from_slice(&v[2].to_le_bytes());
    }
    bin.extend_from_slice(&[0u16.to_le_bytes(), 1u16.to_le_bytes(), 2u16.to_le_bytes()].concat());
    bin
}

/// Assemble a GLB container from a JSON document and a BIN payload.
pub fn assemble_glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json_bytes = json.as_bytes().to_vec();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' '); // GLB requires space padding for the JSON chunk
    }
    let mut bin_padded = bin.to_vec();
    while !bin_padded.len().is_multiple_of(4) {
        bin_padded.push(0);
    }

    let json_chunk_len = json_bytes.len() as u32;
    let bin_chunk_len = bin_padded.len() as u32;
    let total = 12 + 8 + json_chunk_len + 8 + bin_chunk_len;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&json_chunk_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&bin_chunk_len.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin_padded);
    out
}

/// Patch the declared total length field of a GLB (bytes 8..12).
pub fn patch_glb_total_length(glb: &[u8], new_total: u32) -> Vec<u8> {
    let mut out = glb.to_vec();
    out[8..12].copy_from_slice(&new_total.to_le_bytes());
    out
}

/// A minimal valid GLB with an embedded image texture in the BIN chunk.
pub fn minimal_glb_with_image() -> Vec<u8> {
    let png = png_bytes();
    let bin = triangle_bin();
    let image_offset = bin.len() as u32;
    let mut bin_full = bin;
    bin_full.extend_from_slice(&png);

    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}}],
  "materials": [{{"pbrMetallicRoughness": {{"baseColorTexture": {{"index": 0}}}}}}],
  "textures": [{{"source": 0}}],
  "images": [{{"bufferView": 2, "mimeType": "image/png"}}],
  "buffers": [{{"byteLength": {buffer_len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}},
    {{"buffer": 0, "byteOffset": {image_offset}, "byteLength": {png_len}}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
        buffer_len = bin_full.len(),
        png_len = png.len(),
    );
    assemble_glb(&json, &bin_full)
}

/// A minimal valid GLB without textures.
pub fn minimal_glb_plain() -> Vec<u8> {
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
  "buffers": [{{"byteLength": {buffer_len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
        buffer_len = triangle_bin().len(),
    );
    assemble_glb(&json, &triangle_bin())
}

pub struct GltfFixture {
    pub source_path: PathBuf,
}

/// Write a plain `.gltf` with an external `.bin` buffer and an external
/// texture. `buffer_uri` / `image_uri` can be overridden for negative tests.
pub fn write_gltf_fixture(
    dir: &Path,
    buffer_uri: &str,
    image_uri: Option<&str>,
    write_deps: bool,
) -> GltfFixture {
    let bin = triangle_bin();
    let png = png_bytes();
    if write_deps && buffer_uri.starts_with("data:") {
        // data: URI buffers need no file, but keep the directory realistic.
    }
    let images_json = match image_uri {
        Some(uri) => format!(r#""images": [{{"uri": "{uri}"}}],"#),
        None => String::new(),
    };
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
  {images_json}
  "buffers": [{{"uri": "{buffer_uri}", "byteLength": {buffer_len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
        buffer_len = bin.len(),
    );
    fs::write(dir.join("scene.gltf"), json).expect("write gltf");

    if write_deps && !buffer_uri.starts_with("data:") {
        let p = dir.join(buffer_uri);
        fs::create_dir_all(p.parent().unwrap()).expect("create buffer dir");
        fs::write(p, &bin).expect("write bin");
    }
    if write_deps {
        if let Some(uri) = image_uri {
            let p = dir.join(uri);
            fs::create_dir_all(p.parent().unwrap()).expect("create texture dir");
            fs::write(p, &png).expect("write texture");
        }
    }
    GltfFixture {
        source_path: dir.join("scene.gltf"),
    }
}

pub struct ObjFixture {
    pub source_path: PathBuf,
}

/// Write an OBJ + MTL + texture fixture.
pub fn write_obj_fixture(dir: &Path) -> ObjFixture {
    let obj = "# test fixture\n\
               v 0 0 0\nv 1 0 0\nv 0 1 0\n\
               vt 0 0\nvn 0 0 1\n\
               f 1/1/1 2/1/1 3/1/1\n\
               mtllib materials.mtl\n";
    let mtl = "newmtl body\nKd 0.8 0.8 0.8\nmap_Kd textures/albedo.png\n";
    fs::write(dir.join("model.obj"), obj).expect("write obj");
    fs::write(dir.join("materials.mtl"), mtl).expect("write mtl");
    let tex = dir.join("textures").join("albedo.png");
    fs::create_dir_all(tex.parent().unwrap()).expect("create textures dir");
    fs::write(tex, png_bytes()).expect("write texture");
    ObjFixture {
        source_path: dir.join("model.obj"),
    }
}

/// Percent-encode a base64 payload for a `data:` URI buffer.
pub fn data_uri_buffer() -> String {
    let bin = triangle_bin();
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:application/octet-stream;base64,");
    for chunk in bin.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(CHARS[(n >> 18) as usize & 63] as char);
        out.push(CHARS[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            CHARS[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A minimal GLB whose single buffer is a `data:` URI instead of the BIN
/// chunk (allowed by the glTF spec for buffers; images may also use URIs).
pub fn minimal_glb_with_data_uri_buffer() -> Vec<u8> {
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
  "buffers": [{{"uri": "{uri}", "byteLength": {buffer_len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
        uri = data_uri_buffer(),
        buffer_len = triangle_bin().len(),
    );
    assemble_glb(&json, &[])
}
