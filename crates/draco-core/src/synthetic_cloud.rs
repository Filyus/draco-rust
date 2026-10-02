//! Point clouds made in code, shaped like the captures a coder meets and like
//! the extremes it rarely does.
//!
//! Each cloud is a function of a point count and a seed, so a failure names
//! its input in two numbers. (The scanned shapes take `sin` and `cos` from the
//! platform's maths library, so their last bits may differ between platforms;
//! the noise does not.) The shapes:
//!
//! - `uniform`: noise in a cube, the worst case for every prediction;
//! - `aerial`: an airborne lidar strip, scan line by scan line, with
//!   intensity, return number, classification and a `f64` time;
//! - `spinning`: one frame of a rotating lidar in firing order, ring inside
//!   azimuth, with the zeros a sensor writes where nothing returned;
//! - `terrestrial`: a station scanning a room, colour and normals by surface;
//! - `lattice`: an integer grid with every point four times, in random order;
//! - `far_away`: a metre of points a million metres out, where `f32` has
//!   already rounded the positions to a few steps;
//! - `constant`: one value in every attribute of every point;
//! - `skewed`: integer attributes at the edges of a symbol coder -- almost
//!   all one value, `i32` extremes, an alphabet past 2^16, all of `u16`;
//! - `splat`: a Gaussian splat's 58 attributes, with its zero normals.
//!
//! The crate's tests build a `PointCloud` from these; the
//! `synthetic_clouds` example includes this file to write them as PLY for
//! timing. Only `crate::` paths that both resolve are used for that reason.

use crate::{DataType, GeometryAttributeType, PointAttribute, PointCloud};

/// One attribute's values, point after point, components inside.
pub enum Values {
    F32(Vec<f32>),
    F64(Vec<f64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    I32(Vec<i32>),
}

impl Values {
    pub fn data_type(&self) -> DataType {
        match self {
            Values::F32(_) => DataType::Float32,
            Values::F64(_) => DataType::Float64,
            Values::U8(_) => DataType::Uint8,
            Values::U16(_) => DataType::Uint16,
            Values::I32(_) => DataType::Int32,
        }
    }

    /// The values as little-endian bytes, in order.
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            Values::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Values::F64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Values::U8(v) => v.clone(),
            Values::U16(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Values::I32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    }

    fn permuted(&self, order: &[u32], components: usize) -> Values {
        fn take<T: Copy>(v: &[T], order: &[u32], components: usize) -> Vec<T> {
            order
                .iter()
                .flat_map(|&p| {
                    let start = p as usize * components;
                    v[start..start + components].iter().copied()
                })
                .collect()
        }
        match self {
            Values::F32(v) => Values::F32(take(v, order, components)),
            Values::F64(v) => Values::F64(take(v, order, components)),
            Values::U8(v) => Values::U8(take(v, order, components)),
            Values::U16(v) => Values::U16(take(v, order, components)),
            Values::I32(v) => Values::I32(take(v, order, components)),
        }
    }
}

/// One attribute. A generic attribute has one component and is the PLY
/// property `name`; position, normal and colour have three and take PLY's own
/// names for them.
pub struct Column {
    pub name: &'static str,
    pub kind: GeometryAttributeType,
    pub components: usize,
    pub values: Values,
    /// Quantization bits for a `f32` attribute; `0` leaves it unquantized.
    pub quantization_bits: i32,
}

pub struct Cloud {
    pub name: String,
    pub points: usize,
    pub columns: Vec<Column>,
}

impl Cloud {
    pub fn to_point_cloud(&self) -> PointCloud {
        let mut cloud = PointCloud::new();
        cloud.set_num_points(self.points);
        for column in &self.columns {
            let mut attribute = PointAttribute::new();
            attribute.init(
                column.kind,
                column.components as u8,
                column.values.data_type(),
                false,
                self.points,
            );
            attribute.buffer_mut().write(0, &column.values.bytes());
            cloud.add_attribute(attribute);
        }
        cloud
    }

    /// The same cloud with its points in random order.
    pub fn shuffled(&self, seed: u64) -> Cloud {
        let mut rng = Rng::new(seed);
        let mut order: Vec<u32> = (0..self.points as u32).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        Cloud {
            name: format!("{}_shuffled", self.name),
            points: self.points,
            columns: self
                .columns
                .iter()
                .map(|column| Column {
                    name: column.name,
                    kind: column.kind,
                    components: column.components,
                    values: column.values.permuted(&order, column.components),
                    quantization_bits: column.quantization_bits,
                })
                .collect(),
        }
    }
}

/// Every shape, `points` points each.
pub fn all(points: usize, seed: u64) -> Vec<Cloud> {
    vec![
        uniform(points, seed),
        aerial(points, seed),
        spinning(points, seed),
        terrestrial(points, seed),
        lattice(points, seed),
        far_away(points, seed),
        constant(points),
        skewed(points, seed),
        splat(points, seed),
    ]
}

/// xorshift64*: fixed, fast and the same everywhere.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        ((self.next() >> 32) * n) >> 32
    }

    /// Roughly standard normal: a sum of four uniforms, rescaled. No
    /// transcendental call, so the values do not depend on the platform's
    /// maths library.
    fn normal(&mut self) -> f32 {
        let sum = self.unit() + self.unit() + self.unit() + self.unit();
        (sum - 2.0) * 1.732_050_8
    }
}

fn position(values: Vec<f32>, bits: i32) -> Column {
    Column {
        name: "position",
        kind: GeometryAttributeType::Position,
        components: 3,
        values: Values::F32(values),
        quantization_bits: bits,
    }
}

fn colour(values: Vec<u8>) -> Column {
    Column {
        name: "colour",
        kind: GeometryAttributeType::Color,
        components: 3,
        values: Values::U8(values),
        quantization_bits: 0,
    }
}

fn normal(values: Vec<f32>, bits: i32) -> Column {
    Column {
        name: "normal",
        kind: GeometryAttributeType::Normal,
        components: 3,
        values: Values::F32(values),
        quantization_bits: bits,
    }
}

fn generic(name: &'static str, values: Values, bits: i32) -> Column {
    Column {
        name,
        kind: GeometryAttributeType::Generic,
        components: 1,
        values,
        quantization_bits: bits,
    }
}

fn cloud(name: &str, points: usize, columns: Vec<Column>) -> Cloud {
    Cloud {
        name: name.to_string(),
        points,
        columns,
    }
}

pub fn uniform(points: usize, seed: u64) -> Cloud {
    let mut rng = Rng::new(seed);
    let xyz = (0..points * 3).map(|_| rng.unit() * 100.0).collect();
    let rgb = (0..points * 3).map(|_| (rng.next() >> 56) as u8).collect();
    cloud("uniform", points, vec![position(xyz, 16), colour(rgb)])
}

/// Ground under a strip of airborne lidar: hills, flat-roofed blocks and
/// trees, at `(x, y)` in metres.
fn ground(x: f32, y: f32) -> f32 {
    50.0 + 10.0 * (x / 37.0).sin() * (y / 53.0).cos()
}

fn is_building(x: f32, y: f32) -> bool {
    let (bx, by) = ((x / 25.0).floor() as i32, (y / 25.0).floor() as i32);
    (bx * 7 + by * 13).rem_euclid(9) == 0 && x.rem_euclid(25.0) > 4.0 && y.rem_euclid(25.0) > 4.0
}

fn is_tree(x: f32, y: f32) -> bool {
    let (bx, by) = ((x / 8.0).floor() as i32, (y / 8.0).floor() as i32);
    (bx * 31 + by * 17).rem_euclid(5) == 0
}

pub fn aerial(points: usize, seed: u64) -> Cloud {
    // A line of 400 shots sweeps 200 m across the track, there and back, and
    // the aircraft moves half a metre a line.
    const LINE: usize = 400;
    let mut rng = Rng::new(seed);
    let mut xyz = Vec::with_capacity(points * 3);
    let mut intensity = Vec::with_capacity(points);
    let mut returns = Vec::with_capacity(points);
    let mut class = Vec::with_capacity(points);
    let mut time = Vec::with_capacity(points);
    let mut rgb = Vec::with_capacity(points * 3);
    for i in 0..points {
        let (line, shot) = (i / LINE, i % LINE);
        let across = if line % 2 == 0 { shot } else { LINE - 1 - shot };
        let x = line as f32 * 0.5 + rng.normal() * 0.05;
        let y = across as f32 * 0.5 + rng.normal() * 0.05;
        let floor = ground(x, y);
        let (z, c, r, base) = if is_building(x, y) {
            (floor + 12.0, 6u8, 1u8, 900.0)
        } else if is_tree(x, y) && rng.unit() < 0.7 {
            let hit = rng.unit();
            (floor + 3.0 + 9.0 * hit, 5, 1 + (hit * 3.0) as u8, 400.0)
        } else {
            (floor, 2, 1, 1500.0)
        };
        xyz.extend([x, y, z + rng.normal() * 0.03]);
        intensity.push((base + rng.normal() * 120.0).clamp(0.0, 65535.0) as u16);
        returns.push(r);
        class.push(c);
        time.push(3.0e8 + i as f64 * 1.0e-5);
        let shade = (c as f32 * 30.0 + z) as u8;
        rgb.extend([shade, shade.wrapping_add(20), shade / 2]);
    }
    cloud(
        "aerial",
        points,
        vec![
            position(xyz, 16),
            colour(rgb),
            generic("intensity", Values::U16(intensity), 0),
            generic("return_number", Values::U8(returns), 0),
            generic("classification", Values::U8(class), 0),
            generic("gps_time", Values::F64(time), 0),
        ],
    )
}

pub fn spinning(points: usize, seed: u64) -> Cloud {
    // 64 rings fired together at each azimuth, from 25 degrees down to 3 up;
    // the sensor stands 1.8 m over flat ground inside walls about 25 m away.
    const RINGS: usize = 64;
    let mut rng = Rng::new(seed);
    let steps = points.div_ceil(RINGS).max(1);
    let mut xyz = Vec::with_capacity(points * 3);
    let mut intensity = Vec::with_capacity(points);
    let mut ring = Vec::with_capacity(points);
    for i in 0..points {
        let (step, r) = (i / RINGS, i % RINGS);
        let azimuth = step as f64 / steps as f64 * std::f64::consts::TAU;
        let elevation = (-25.0 + 28.0 * r as f64 / (RINGS - 1) as f64).to_radians();
        let wall = 25.0 + 5.0 * (3.0 * azimuth).sin();
        let to_ground = if elevation < 0.0 {
            1.8 / -elevation.tan()
        } else {
            f64::INFINITY
        };
        let range = to_ground.min(wall / elevation.cos());
        // Nothing returns past 100 m, and a few shots in a hundred are lost
        // anyway; the sensor writes zeros for both.
        if range > 100.0 || rng.unit() < 0.03 {
            xyz.extend([0.0f32; 3]);
            intensity.push(0);
        } else {
            let range = range + f64::from(rng.normal()) * 0.02;
            let flat = range * elevation.cos();
            xyz.extend([
                (flat * azimuth.cos()) as f32,
                (flat * azimuth.sin()) as f32,
                (range * elevation.sin()) as f32,
            ]);
            intensity.push((200.0 - range * 4.0).clamp(1.0, 255.0) as u8);
        }
        ring.push(r as u16);
    }
    cloud(
        "spinning",
        points,
        vec![
            position(xyz, 14),
            generic("intensity", Values::U8(intensity), 0),
            generic("ring", Values::U16(ring), 0),
        ],
    )
}

pub fn terrestrial(points: usize, seed: u64) -> Cloud {
    // A station at the middle of a 10 x 8 x 3 m room, 1.5 m up, scanning a
    // column of elevations at each azimuth, with a ball on the floor.
    const COLUMN: usize = 500;
    let mut rng = Rng::new(seed);
    let columns = points.div_ceil(COLUMN).max(1);
    let half = [5.0f32, 4.0, 1.5];
    let ball = ([2.0f32, 1.0, -1.0], 0.5f32);
    let mut xyz = Vec::with_capacity(points * 3);
    let mut normals = Vec::with_capacity(points * 3);
    let mut rgb = Vec::with_capacity(points * 3);
    for i in 0..points {
        let (column, row) = (i / COLUMN, i % COLUMN);
        let azimuth = column as f32 / columns as f32 * std::f32::consts::TAU;
        let elevation = (-80.0 + 160.0 * row as f32 / (COLUMN - 1) as f32).to_radians();
        let d = [
            elevation.cos() * azimuth.cos(),
            elevation.cos() * azimuth.sin(),
            elevation.sin(),
        ];
        // The nearest wall, floor or ceiling along `d`.
        let (mut t, mut axis) = (f32::INFINITY, 0);
        for (k, (&dk, &hk)) in d.iter().zip(&half).enumerate() {
            if dk != 0.0 {
                let tk = hk / dk.abs();
                if tk < t {
                    (t, axis) = (tk, k);
                }
            }
        }
        let mut n = [0.0f32; 3];
        n[axis] = -d[axis].signum();
        let mut surface = axis as u8;
        // The ball, if the ray meets it first.
        let (c, radius) = ball;
        let b = d[0] * c[0] + d[1] * c[1] + d[2] * c[2];
        let disc = b * b - (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]) + radius * radius;
        if disc > 0.0 && b - disc.sqrt() > 0.0 && b - disc.sqrt() < t {
            t = b - disc.sqrt();
            n = [
                (d[0] * t - c[0]) / radius,
                (d[1] * t - c[1]) / radius,
                (d[2] * t - c[2]) / radius,
            ];
            surface = 3;
        }
        let t = t + rng.normal() * 0.002;
        let p = [d[0] * t, d[1] * t, d[2] * t];
        xyz.extend(p);
        normals.extend(n);
        let base = [
            [200u8, 190, 170],
            [180, 60, 50],
            [90, 90, 100],
            [30, 120, 200],
        ][surface as usize];
        let light = (p[2] + 1.5) * 15.0;
        for channel in base {
            rgb.push((f32::from(channel) + light + rng.normal() * 4.0).clamp(0.0, 255.0) as u8);
        }
    }
    cloud(
        "terrestrial",
        points,
        vec![position(xyz, 16), normal(normals, 8), colour(rgb)],
    )
}

pub fn lattice(points: usize, seed: u64) -> Cloud {
    let mut rng = Rng::new(seed);
    let cells = points.div_ceil(4).max(1);
    let side = (cells as f64).cbrt().ceil() as usize;
    let mut order: Vec<usize> = (0..points).map(|i| i / 4).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let mut xyz = Vec::with_capacity(points * 3);
    let mut id = Vec::with_capacity(points);
    for &cell in &order {
        xyz.extend([
            (cell % side) as f32,
            (cell / side % side) as f32,
            (cell / side / side) as f32,
        ]);
        id.push(cell as i32);
    }
    cloud(
        "lattice",
        points,
        vec![position(xyz, 11), generic("cell", Values::I32(id), 0)],
    )
}

pub fn far_away(points: usize, seed: u64) -> Cloud {
    let mut rng = Rng::new(seed);
    let origin = [1.0e6f32, 2.0e6, 3.0e3];
    let xyz = (0..points * 3)
        .map(|k| origin[k % 3] + rng.unit())
        .collect();
    let height = (0..points).map(|_| rng.unit()).collect();
    cloud(
        "far_away",
        points,
        vec![
            position(xyz, 16),
            generic("height", Values::F32(height), 12),
        ],
    )
}

pub fn constant(points: usize) -> Cloud {
    cloud(
        "constant",
        points,
        vec![
            position([1.0f32, 2.0, 3.0].repeat(points), 16),
            normal([0.0f32, 0.0, 1.0].repeat(points), 8),
            colour([10u8, 20, 30].repeat(points)),
            generic("zero", Values::F32(vec![0.0; points]), 8),
            generic("seven", Values::I32(vec![7; points]), 0),
        ],
    )
}

pub fn skewed(points: usize, seed: u64) -> Cloud {
    let mut rng = Rng::new(seed);
    let xyz = (0..points * 3).map(|_| rng.unit()).collect();
    let sparse = (0..points)
        .map(|_| {
            if rng.unit() < 0.999 {
                0
            } else if rng.unit() < 0.5 {
                i32::MIN
            } else {
                i32::MAX
            }
        })
        .collect();
    let wide = (0..points).map(|_| rng.below(1 << 20) as i32).collect();
    // Mostly a step of 100,000 one way or the other, and the rest anywhere in
    // a range wide enough that the wrap transform leaves the step whole: two
    // residual symbols past 2^17, inside the 18 bits the raw coder takes, hold
    // nearly all the probability of a wide alphabet -- a fine rANS table that
    // is summarized in buckets, most of them owned by a large symbol.
    let bimodal = (0..points)
        .map(|i| {
            if rng.unit() < 0.95 {
                (i % 2) as i32 * 100_000
            } else {
                rng.below(230_000) as i32
            }
        })
        .collect();
    let few = (0..points).map(|_| rng.below(4) as u8).collect();
    let full = (0..points).map(|_| (rng.next() >> 48) as u16).collect();
    cloud(
        "skewed",
        points,
        vec![
            position(xyz, 12),
            generic("sparse", Values::I32(sparse), 0),
            generic("wide", Values::I32(wide), 0),
            generic("bimodal", Values::I32(bimodal), 0),
            generic("few", Values::U8(few), 0),
            generic("full", Values::U16(full), 0),
        ],
    )
}

/// The 3DGS property names past position and normal, in file order.
const SPLAT_PROPERTIES: [&str; 56] = [
    "f_dc_0",
    "f_dc_1",
    "f_dc_2",
    "f_rest_0",
    "f_rest_1",
    "f_rest_2",
    "f_rest_3",
    "f_rest_4",
    "f_rest_5",
    "f_rest_6",
    "f_rest_7",
    "f_rest_8",
    "f_rest_9",
    "f_rest_10",
    "f_rest_11",
    "f_rest_12",
    "f_rest_13",
    "f_rest_14",
    "f_rest_15",
    "f_rest_16",
    "f_rest_17",
    "f_rest_18",
    "f_rest_19",
    "f_rest_20",
    "f_rest_21",
    "f_rest_22",
    "f_rest_23",
    "f_rest_24",
    "f_rest_25",
    "f_rest_26",
    "f_rest_27",
    "f_rest_28",
    "f_rest_29",
    "f_rest_30",
    "f_rest_31",
    "f_rest_32",
    "f_rest_33",
    "f_rest_34",
    "f_rest_35",
    "f_rest_36",
    "f_rest_37",
    "f_rest_38",
    "f_rest_39",
    "f_rest_40",
    "f_rest_41",
    "f_rest_42",
    "f_rest_43",
    "f_rest_44",
    "opacity",
    "scale_0",
    "scale_1",
    "scale_2",
    "rot_0",
    "rot_1",
    "rot_2",
    "rot_3",
];

pub fn splat(points: usize, seed: u64) -> Cloud {
    // Splats crowd around a few dozen centres; their colour harmonics shrink
    // band by band, their scales are log-normal and their rotations unit
    // quaternions. The normals are zero, as a 3DGS file's are.
    let mut rng = Rng::new(seed);
    let centres: Vec<[f32; 3]> = (0..64)
        .map(|_| [rng.unit() * 40.0, rng.unit() * 40.0, rng.unit() * 10.0])
        .collect();
    let mut xyz = Vec::with_capacity(points * 3);
    for _ in 0..points {
        let c = centres[rng.below(64) as usize];
        let spread = 0.5 + rng.unit() * 3.0;
        xyz.extend(c.map(|v| v + rng.normal() * spread));
    }
    let mut columns = vec![position(xyz, 16), normal(vec![0.0; points * 3], 8)];
    let rotation: Vec<[f32; 4]> = (0..points)
        .map(|_| {
            let q = [rng.normal(), rng.normal(), rng.normal(), rng.normal()];
            let length = (q.iter().map(|v| v * v).sum::<f32>()).sqrt().max(1e-6);
            q.map(|v| v / length)
        })
        .collect();
    for (k, name) in SPLAT_PROPERTIES.iter().enumerate() {
        let values: Vec<f32> = match *name {
            "opacity" => (0..points).map(|_| 2.0 + rng.normal() * 2.0).collect(),
            "scale_0" | "scale_1" | "scale_2" => (0..points).map(|_| -4.0 + rng.normal()).collect(),
            _ if name.starts_with("rot_") => {
                let axis = usize::from(name.as_bytes()[4] - b'0');
                rotation.iter().map(|q| q[axis]).collect()
            }
            _ if k < 3 => (0..points).map(|_| rng.normal() * 0.8).collect(),
            _ => {
                let band = 1.0 + ((k - 3) % 15) as f32 / 5.0;
                (0..points).map(|_| rng.normal() * 0.05 / band).collect()
            }
        };
        let bits = if name.starts_with("f_rest_") { 6 } else { 8 };
        columns.push(generic(name, Values::F32(values), bits));
    }
    cloud("splat", points, columns)
}
