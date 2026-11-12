// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small glTF 2.0 writer for binary files (`.glb`): meshes of indexed triangles with normals and UVs, materials, textures kept as
//! separate WebP files (`EXT_texture_webp`, no fallback image), nodes with names and `extras`, and instanced nodes
//! (`EXT_mesh_gpu_instancing`). Only what `area-3d` needs; everything is written in the order it was added, so the same calls give
//! the same bytes. [`read_glb`] reads a file back (for checks and tests).

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

/// glTF's buffer view targets.
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// Component types.
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

/// The glTF extension for instanced nodes, which three.js's `GLTFLoader` reads into `InstancedMesh`es.
pub const INSTANCING: &str = "EXT_mesh_gpu_instancing";
/// The glTF extension for WebP textures.
pub const WEBP: &str = "EXT_texture_webp";

/// A triangle's corners: positions and UVs.
pub type Corners = ([[f32; 3]; 3], [[f32; 2]; 3]);

/// One primitive of a mesh: indexed triangles, positions, normals and UVs (glTF's axes and UV origin), and its material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Primitive {
    /// Each vertex's position, in glTF's axes (Y up), metres.
    pub positions: Vec<[f32; 3]>,
    /// Each vertex's normal, of length 1.
    pub normals: Vec<[f32; 3]>,
    /// Each vertex's texture coordinates, v down (glTF's origin is the image's top left).
    pub uvs: Vec<[f32; 2]>,
    /// Three vertex indices a triangle, counter-clockwise seen from the front.
    pub indices: Vec<u32>,
    /// Its material's index; None for glTF's default material.
    pub material: Option<usize>,
}

impl Primitive {
    /// Triangles (corners' positions and UVs) as one indexed primitive: corners with the same position and UV, bit for bit, are
    /// one vertex (the first time seen gives the order). Each vertex's normal is the sum of its triangles' face normals weighted by
    /// their areas, normalised (as three.js's `computeVertexNormals` makes them on an indexed geometry); straight up (+Y) where
    /// the sum is nothing.
    pub fn welded(triangles: impl IntoIterator<Item = Corners>, material: Option<usize>) -> Primitive {
        let mut out = Primitive { material, ..Default::default() };
        let mut seen: HashMap<([u32; 3], [u32; 2]), u32> = HashMap::new();
        let mut sums: Vec<[f64; 3]> = Vec::new();
        for (p, uv) in triangles {
            let n = face_normal(&p);
            for k in 0..3 {
                let key = (p[k].map(f32::to_bits), uv[k].map(f32::to_bits));
                let i = *seen.entry(key).or_insert_with(|| {
                    out.positions.push(p[k]);
                    out.uvs.push(uv[k]);
                    sums.push([0.0; 3]);
                    (out.positions.len() - 1) as u32
                });
                for c in 0..3 {
                    sums[i as usize][c] += n[c];
                }
                out.indices.push(i);
            }
        }
        out.normals = sums
            .iter()
            .map(|s| {
                let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
                if len > 1e-12 { [(s[0] / len) as f32, (s[1] / len) as f32, (s[2] / len) as f32] } else { [0.0, 1.0, 0.0] }
            })
            .collect();
        out
    }

    /// How many triangles it has.
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A triangle's normal, its length twice its area (right-handed: counter-clockwise corners face the viewer).
fn face_normal(p: &[[f32; 3]; 3]) -> [f64; 3] {
    let d = |a: [f32; 3], b: [f32; 3]| {
        [f64::from(b[0]) - f64::from(a[0]), f64::from(b[1]) - f64::from(a[1]), f64::from(b[2]) - f64::from(a[2])]
    };
    let (u, v) = (d(p[0], p[1]), d(p[0], p[2]));
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

/// A glTF file being written.
#[derive(Default)]
pub struct Gltf {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    materials: Vec<Value>,
    textures: Vec<Value>,
    images: Vec<Value>,
    samplers: Vec<Value>,
    nodes: Vec<Value>,
    roots: Vec<usize>,
    used: BTreeSet<&'static str>,
    required: BTreeSet<&'static str>,
    generator: String,
}

impl Gltf {
    /// An empty file whose `asset.generator` will be `generator`.
    pub fn new(generator: &str) -> Gltf {
        Gltf { generator: generator.to_string(), ..Default::default() }
    }

    /// `bytes` as a buffer view (aligned to 4 bytes), its index.
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    /// Floats, `N` a element, as an accessor of `kind` (`VEC2`, `VEC3`, `VEC4`); with their bounds when `bounds` (positions need them).
    fn floats<const N: usize>(&mut self, data: &[[f32; N]], kind: &str, target: Option<u32>, bounds: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        let mut a = json!({ "bufferView": view, "componentType": FLOAT, "count": data.len(), "type": kind });
        if bounds && !data.is_empty() {
            let (mut lo, mut hi) = ([f32::INFINITY; N], [f32::NEG_INFINITY; N]);
            for v in data {
                for c in 0..N {
                    lo[c] = lo[c].min(v[c]);
                    hi[c] = hi[c].max(v[c]);
                }
            }
            a["min"] = json!(lo.to_vec());
            a["max"] = json!(hi.to_vec());
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    /// Triangle indices: 16-bit when every index fits, else 32-bit.
    fn indices(&mut self, idx: &[u32]) -> usize {
        let short = idx.iter().all(|&i| i < u32::from(u16::MAX));
        let bytes: Vec<u8> = if short {
            idx.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect()
        } else {
            idx.iter().flat_map(|i| i.to_le_bytes()).collect()
        };
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessors.push(json!({ "bufferView": view, "componentType": if short { UNSIGNED_SHORT } else { UNSIGNED_INT }, "count": idx.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }

    /// A mesh of `primitives`, its index.
    pub fn mesh(&mut self, name: &str, primitives: &[Primitive]) -> usize {
        let prims: Vec<Value> = primitives
            .iter()
            .map(|p| {
                let position = self.floats(&p.positions, "VEC3", Some(ARRAY_BUFFER), true);
                let normal = self.floats(&p.normals, "VEC3", Some(ARRAY_BUFFER), false);
                let uv = self.floats(&p.uvs, "VEC2", Some(ARRAY_BUFFER), false);
                let indices = self.indices(&p.indices);
                let mut v =
                    json!({ "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv }, "indices": indices, "mode": 4 });
                if let Some(m) = p.material {
                    v["material"] = json!(m);
                }
                v
            })
            .collect();
        self.meshes.push(json!({ "name": name, "primitives": prims }));
        self.meshes.len() - 1
    }

    /// A material (a glTF material object), its index.
    pub fn material(&mut self, m: Value) -> usize {
        self.materials.push(m);
        self.materials.len() - 1
    }

    /// A texture whose image is the WebP file at `uri` (relative to the `.glb`), repeated, mip-mapped; its index.
    pub fn webp_texture(&mut self, uri: &str) -> usize {
        if self.samplers.is_empty() {
            // LINEAR, LINEAR_MIPMAP_LINEAR, REPEAT both ways.
            self.samplers.push(json!({ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }));
        }
        self.images.push(json!({ "uri": uri, "mimeType": "image/webp" }));
        self.textures.push(json!({ "sampler": 0, "extensions": { WEBP: { "source": self.images.len() - 1 } } }));
        self.used.insert(WEBP);
        // (No PNG fallback: a loader that can't read WebP can't show the file.)
        self.required.insert(WEBP);
        self.textures.len() - 1
    }

    /// A node (a glTF node object), its index.
    pub fn node(&mut self, n: Value) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    /// A node drawing `mesh` once for each of `translations` and `rotations` (unit quaternions x, y, z, w), with
    /// `EXT_mesh_gpu_instancing`; its index.
    pub fn instanced(&mut self, name: &str, mesh: usize, translations: &[[f32; 3]], rotations: &[[f32; 4]], extras: Value) -> usize {
        assert_eq!(translations.len(), rotations.len(), "one rotation an instance");
        let t = self.floats(translations, "VEC3", None, true);
        let r = self.floats(rotations, "VEC4", None, false);
        self.used.insert(INSTANCING);
        // (Without the extension a loader would draw one copy at the origin: required.)
        self.required.insert(INSTANCING);
        self.node(json!({ "name": name, "mesh": mesh, "extensions": { INSTANCING: { "attributes": { "TRANSLATION": t, "ROTATION": r } } }, "extras": extras }))
    }

    /// Node `n` is one of the scene's roots.
    pub fn root(&mut self, n: usize) {
        self.roots.push(n);
    }

    /// Bytes in the buffer so far.
    pub fn bin_len(&self) -> usize {
        self.bin.len()
    }

    /// The JSON part.
    pub fn json(&self) -> Value {
        let mut j = Map::new();
        j.insert("asset".into(), json!({ "version": "2.0", "generator": self.generator }));
        if !self.used.is_empty() {
            j.insert("extensionsUsed".into(), json!(self.used));
        }
        if !self.required.is_empty() {
            j.insert("extensionsRequired".into(), json!(self.required));
        }
        j.insert("scene".into(), json!(0));
        j.insert("scenes".into(), json!([{ "nodes": self.roots }]));
        for (name, list) in [
            ("nodes", &self.nodes),
            ("meshes", &self.meshes),
            ("materials", &self.materials),
            ("textures", &self.textures),
            ("images", &self.images),
            ("samplers", &self.samplers),
            ("accessors", &self.accessors),
            ("bufferViews", &self.views),
        ] {
            if !list.is_empty() {
                j.insert(name.into(), Value::Array(list.clone()));
            }
        }
        if !self.bin.is_empty() {
            j.insert("buffers".into(), json!([{ "byteLength": self.padded_bin_len() }]));
        }
        Value::Object(j)
    }

    fn padded_bin_len(&self) -> usize {
        self.bin.len().next_multiple_of(4)
    }

    /// The `.glb`: header, JSON chunk (padded with spaces), BIN chunk (padded with zeros).
    pub fn to_glb(&self) -> Vec<u8> {
        let mut text = serde_json::to_vec(&self.json()).expect("JSON");
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let bin_len = if self.bin.is_empty() { 0 } else { 8 + self.padded_bin_len() };
        let total = 12 + 8 + text.len() + bin_len;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        if !self.bin.is_empty() {
            out.extend_from_slice(&(self.padded_bin_len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&self.bin);
            out.resize(total, 0);
        }
        out
    }
}

/// A `.glb`'s JSON and BIN chunks (the BIN empty when there is none), checked for the header and chunk lengths.
pub fn read_glb(bytes: &[u8]) -> Result<(Value, Vec<u8>)> {
    let u32_at = |i: usize| -> Result<u32> { Ok(u32::from_le_bytes(bytes.get(i..i + 4).context("short file")?.try_into()?)) };
    ensure!(bytes.get(0..4) == Some(b"glTF"), "not a glb");
    ensure!(u32_at(4)? == 2, "not glTF 2");
    ensure!(u32_at(8)? as usize == bytes.len(), "the header's length is not the file's");
    let json_len = u32_at(12)? as usize;
    ensure!(bytes.get(16..20) == Some(b"JSON"), "the first chunk is not JSON");
    let json: Value = serde_json::from_slice(bytes.get(20..20 + json_len).context("short JSON chunk")?)?;
    let at = 20 + json_len;
    if at == bytes.len() {
        return Ok((json, Vec::new()));
    }
    let bin_len = u32_at(at)? as usize;
    if bytes.get(at + 4..at + 8) != Some(b"BIN\0") {
        bail!("the second chunk is not BIN");
    }
    let bin = bytes.get(at + 8..at + 8 + bin_len).context("short BIN chunk")?.to_vec();
    Ok((json, bin))
}

/// Accessor `a`'s floats from a file's JSON and BIN ([`read_glb`]), `N` a element (tightly packed, as [`Gltf`] writes them).
pub fn read_floats<const N: usize>(json: &Value, bin: &[u8], a: usize) -> Result<Vec<[f32; N]>> {
    let acc = &json["accessors"][a];
    ensure!(acc["componentType"] == FLOAT, "accessor {a} is not float");
    let view = &json["bufferViews"][acc["bufferView"].as_u64().context("no view")? as usize];
    let (off, count) = (view["byteOffset"].as_u64().unwrap_or(0) as usize, acc["count"].as_u64().context("no count")? as usize);
    let data = bin.get(off..off + count * N * 4).context("accessor past the buffer")?;
    Ok(data
        .chunks_exact(N * 4)
        .map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().expect("a chunk of N * 4 bytes"))))
        .collect())
}

/// Accessor `a`'s indices (16- or 32-bit).
pub fn read_indices(json: &Value, bin: &[u8], a: usize) -> Result<Vec<u32>> {
    let acc = &json["accessors"][a];
    let view = &json["bufferViews"][acc["bufferView"].as_u64().context("no view")? as usize];
    let (off, count) = (view["byteOffset"].as_u64().unwrap_or(0) as usize, acc["count"].as_u64().context("no count")? as usize);
    Ok(match acc["componentType"].as_u64() {
        Some(5123) => bin
            .get(off..off + count * 2)
            .context("past the buffer")?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u32::from(u16::from_le_bytes(*c)))
            .collect(),
        Some(5125) => {
            bin.get(off..off + count * 4).context("past the buffer")?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
        }
        t => bail!("accessor {a}: component type {t:?} is not an index type"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(x: f32) -> Vec<Corners> {
        let (a, b, c, d) = ([x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x + 1.0, 0.0, -1.0], [x, 0.0, -1.0]);
        let (ua, ub, uc, ud) = ([0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]);
        vec![([a, b, c], [ua, ub, uc]), ([a, c, d], [ua, uc, ud])]
    }

    #[test]
    fn corners_with_the_same_position_and_uv_are_one_vertex_and_normals_follow_the_faces() {
        let p = Primitive::welded(quad(0.0), Some(3));
        assert_eq!(p.positions.len(), 4, "a quad is four vertices");
        assert_eq!(p.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(p.triangles(), 2);
        assert_eq!(p.material, Some(3));
        // Counter-clockwise seen from above (+Y), so the normals point up.
        for n in &p.normals {
            assert!((n[1] - 1.0).abs() < 1e-6, "{n:?}");
        }
        // The same position with another UV is another vertex.
        let mut t = quad(0.0);
        t[1].1[0] = [0.5, 0.5];
        assert_eq!(Primitive::welded(t, None).positions.len(), 5);
        // A degenerate triangle has no direction: up.
        let flat = Primitive::welded([([[0.0; 3]; 3], [[0.0; 2]; 3])], None);
        assert_eq!(flat.normals, [[0.0, 1.0, 0.0]]);
    }

    #[test]
    fn a_glb_reads_back_with_its_instances_nodes_extras_and_textures() {
        let mut g = Gltf::new("test");
        let tex = g.webp_texture("tex/0123456789abcdef0123.webp");
        let mat = g.material(json!({ "pbrMetallicRoughness": { "baseColorTexture": { "index": tex } } }));
        let mesh = g.mesh("tile", &[Primitive::welded(quad(0.0), Some(mat))]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let tiles = g.instanced(
            "tiles_a",
            mesh,
            &[[5.0, 0.0, -5.0], [15.0, 2.0, -5.0]],
            &[[0.0, 0.0, 0.0, 1.0], [0.0, half, 0.0, half]],
            json!({ "part": "tiles", "tiles": [0, 1] }),
        );
        let door = g.node(json!({ "name": "door_7", "mesh": mesh, "translation": [1.0, 2.0, 3.0], "extras": { "part": "door", "index": 7, "tag": "CTP_StrongDoor_22" } }));
        let root = g.node(json!({ "name": "area", "children": [tiles, door] }));
        g.root(root);
        let glb = g.to_glb();
        assert_eq!(glb.len() % 4, 0);
        let (j, bin) = read_glb(&glb).unwrap();
        assert_eq!(bin.len(), g.padded_bin_len());
        assert_eq!(j["asset"]["version"], "2.0");
        assert_eq!(j["extensionsUsed"], json!([INSTANCING, WEBP]));
        assert_eq!(j["extensionsRequired"], json!([INSTANCING, WEBP]));
        assert_eq!(j["scenes"][0]["nodes"], json!([root]));
        // The instances: two translations and rotations.
        let inst = &j["nodes"][tiles]["extensions"][INSTANCING]["attributes"];
        let t: Vec<[f32; 3]> = read_floats(&j, &bin, inst["TRANSLATION"].as_u64().unwrap() as usize).unwrap();
        let r: Vec<[f32; 4]> = read_floats(&j, &bin, inst["ROTATION"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(t, [[5.0, 0.0, -5.0], [15.0, 2.0, -5.0]]);
        assert_eq!(r[1], [0.0, half, 0.0, half]);
        assert_eq!(j["nodes"][tiles]["mesh"], json!(mesh));
        // The door: a named node sharing the mesh, with its extras.
        assert_eq!(j["nodes"][door]["name"], "door_7");
        assert_eq!(j["nodes"][door]["mesh"], json!(mesh));
        assert_eq!(j["nodes"][door]["extras"]["tag"], "CTP_StrongDoor_22");
        // The texture: a WebP file outside the glb, through the extension only.
        assert_eq!(j["images"][0], json!({ "uri": "tex/0123456789abcdef0123.webp", "mimeType": "image/webp" }));
        assert_eq!(j["textures"][0]["extensions"][WEBP]["source"], 0);
        assert!(j["textures"][0].get("source").is_none());
        // The mesh's data reads back.
        let prim = &j["meshes"][mesh]["primitives"][0];
        let pos: Vec<[f32; 3]> = read_floats(&j, &bin, prim["attributes"]["POSITION"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(pos.len(), 4);
        assert_eq!(j["accessors"][prim["attributes"]["POSITION"].as_u64().unwrap() as usize]["max"], json!([1.0, 0.0, 0.0]));
        assert_eq!(read_indices(&j, &bin, prim["indices"].as_u64().unwrap() as usize).unwrap(), [0, 1, 2, 0, 2, 3]);
        assert_eq!(j["accessors"][prim["indices"].as_u64().unwrap() as usize]["componentType"], UNSIGNED_SHORT);
        // The same calls give the same bytes.
        assert_eq!(glb, g.to_glb());
    }

    #[test]
    fn many_vertices_take_32_bit_indices_and_a_broken_file_is_refused() {
        let mut g = Gltf::new("test");
        let many: Vec<Corners> =
            (0..22_000).map(|i| ([[i as f32, 0.0, 0.0], [i as f32, 1.0, 0.0], [i as f32, 0.0, 1.0]], [[0.0; 2]; 3])).collect();
        let mesh = g.mesh("big", &[Primitive::welded(many, None)]);
        let n = g.node(json!({ "mesh": mesh }));
        g.root(n);
        let glb = g.to_glb();
        let (j, bin) = read_glb(&glb).unwrap();
        let idx = j["meshes"][0]["primitives"][0]["indices"].as_u64().unwrap() as usize;
        assert_eq!(j["accessors"][idx]["componentType"], UNSIGNED_INT);
        assert_eq!(read_indices(&j, &bin, idx).unwrap().len(), 66_000);
        assert!(read_glb(&glb[..glb.len() - 4]).is_err());
        assert!(read_glb(b"glTF").is_err());
    }
}
