//! Control shapes the user draws or imports, next to the built-in ones in
//! [`crate::controls::Shape`].
//!
//! A custom shape is a few lines in the control's plane, drawn on a 2D grid
//! (across, up), each open or closed, optionally mirrored across the up axis.
//! It is kept by name in the guides file (`custom_shapes`), so the rig builds
//! the same anywhere, and in the user's shape library
//! (`~/.config/hive-rig/shapes.json`) to use on other characters. Drawn, it is
//! fitted to unit size like the built-in shapes, so a control's size means the
//! same for both. Blender and Maya draw its lines from the rig's outline;
//! Unreal, which draws controls from its own meshes, gets the built-in shape
//! that looks most like it ([`CustomShape::look_alike`]).
//!
//! Shapes come in as this crate's JSON (one shape, a bare list of points, or a
//! whole library) or as SVG polylines, polygons, lines and straight paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::controls::{Shape, Strokes};

pub const LIBRARY_FORMAT: &str = "hive-rig-shapes";
pub const LIBRARY_VERSION: u32 = 1;
/// The most points one shape may have, mirrored copies included.
pub const MAX_POINTS: usize = 4096;
/// Line widths are kept between these (1 is each program's normal width).
pub const LINE_WIDTH_RANGE: (f32, f32) = (0.25, 10.0);

/// One line of a custom shape, in the shape's own units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeLine {
    /// Points as `[across, up]`.
    pub points: Vec<[f32; 2]>,
    /// The line runs from its last point back to its first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

/// A control shape drawn by the user. The grid's centre is the control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomShape {
    pub lines: Vec<ShapeLine>,
    /// Every line is mirrored across the up axis (left to right): a line that
    /// ends on the axis joins its mirror image, so half an outline draws whole.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror_x: bool,
}

/// A custom shape with the name it is kept under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedShape {
    pub name: String,
    #[serde(flatten)]
    pub shape: CustomShape,
}

/// How close to the up axis a line's end must be to join its mirror image, as
/// a fraction of the shape's reach.
const ON_AXIS: f32 = 1e-3;

impl CustomShape {
    /// A shape of one line.
    pub fn line(points: Vec<[f32; 2]>, closed: bool, mirror_x: bool) -> CustomShape {
        CustomShape {
            lines: vec![ShapeLine { points, closed }],
            mirror_x,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.lines.is_empty(), "a shape needs at least one line");
        let mut total = 0;
        for (i, line) in self.lines.iter().enumerate() {
            ensure!(
                line.points.len() >= 2,
                "line {} of the shape needs at least two points",
                i + 1
            );
            ensure!(
                line.points.iter().flatten().all(|v| v.is_finite()),
                "line {} of the shape has a point that isn't a number",
                i + 1
            );
            total += line.points.len();
        }
        ensure!(
            total * if self.mirror_x { 2 } else { 1 } <= MAX_POINTS,
            "the shape has too many points ({total}; at most {MAX_POINTS})"
        );
        ensure!(
            self.reach() > 1e-6,
            "the shape has no size: all its points are on the centre"
        );
        Ok(())
    }

    /// The farthest any point is from the centre along either axis.
    fn reach(&self) -> f32 {
        self.lines
            .iter()
            .flat_map(|l| &l.points)
            .flat_map(|p| [p[0].abs(), p[1].abs()])
            .fold(0.0, f32::max)
    }

    /// The lines as drawn, mirrored copies included, in the shape's own units:
    /// `(points, closed)`.
    pub fn drawn_lines(&self) -> Vec<(Vec<Vec2>, bool)> {
        let reach = self.reach().max(1e-6);
        let on_axis = |p: Vec2| p.x.abs() <= ON_AXIS * reach;
        let mut out = Vec::new();
        for line in &self.lines {
            let pts: Vec<Vec2> = line.points.iter().map(|&p| Vec2::from(p)).collect();
            if !self.mirror_x {
                out.push((pts, line.closed));
                continue;
            }
            let flip = |p: &Vec2| Vec2::new(-p.x, p.y);
            let (first, last) = (pts[0], pts[pts.len() - 1]);
            let all_on_axis = pts.iter().all(|p| on_axis(*p));
            if all_on_axis {
                // A line along the axis is its own mirror image.
                out.push((pts, line.closed));
            } else if !line.closed && on_axis(first) && on_axis(last) {
                // Half an outline from the axis round to the axis: the whole loop.
                let mut whole = pts.clone();
                whole.extend(pts.iter().rev().skip(1).take(pts.len() - 2).map(flip));
                out.push((whole, true));
            } else if !line.closed && on_axis(last) {
                let mut whole = pts.clone();
                whole.extend(pts.iter().rev().skip(1).map(flip));
                out.push((whole, false));
            } else if !line.closed && on_axis(first) {
                let mut whole: Vec<Vec2> = pts.iter().rev().map(flip).collect();
                whole.extend(pts.iter().skip(1));
                out.push((whole, false));
            } else {
                out.push((pts.clone(), line.closed));
                out.push((pts.iter().map(flip).collect(), line.closed));
            }
        }
        out
    }

    /// The shape's lines at unit size, in a shape's frame (`x` across, `y`
    /// along the control's axis, `z` up), like [`Shape::strokes`]: the farthest
    /// point is 1 from the centre along across or up, and a closed line repeats
    /// its first point at the end.
    pub fn strokes(&self) -> Strokes {
        let lines = self.drawn_lines();
        let reach = lines
            .iter()
            .flat_map(|(l, _)| l)
            .map(|p| p.abs().max_element())
            .fold(0.0, f32::max)
            .max(1e-6);
        lines
            .into_iter()
            .map(|(pts, closed)| {
                let mut out: Vec<Vec3> = pts
                    .iter()
                    .map(|p| Vec3::new(p.x / reach, 0.0, p.y / reach))
                    .collect();
                if closed && out.len() > 2 {
                    out.push(out[0]);
                }
                out
            })
            .collect()
    }

    /// The built-in shape that looks most like this one, for programs that only
    /// draw their own library of shapes (Unreal). Both are compared as seen
    /// face on, point by point along their lines.
    pub fn look_alike(&self) -> Shape {
        let mine = samples(&self.strokes());
        Shape::ALL
            .into_iter()
            .map(|s| (s, likeness(&mine, &samples(&s.strokes()))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(Shape::Circle, |(s, _)| s)
    }

    /// Read a shape from a file: `.svg` or JSON (see [`CustomShape::from_json`]).
    /// Returns the name the file suggests (its stem) with the shape.
    pub fn import(path: &Path) -> Result<(String, CustomShape)> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let is_svg = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
            || text.trim_start().starts_with('<');
        let shape = if is_svg {
            CustomShape::from_svg(&text)
        } else {
            CustomShape::from_json(&text)
        }
        .with_context(|| format!("reading the shape in {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = name
            .split('.')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("shape");
        Ok((clean_name(stem), shape))
    }

    /// A shape from JSON: `{"lines": [{"points": [[x, y], ...], "closed": true}],
    /// "mirror_x": false}`, one line as `{"points": [...], "closed": ..}`, or a
    /// bare list of `[x, y]` points (an open line). Points may have a third
    /// number, which is ignored.
    pub fn from_json(text: &str) -> Result<CustomShape> {
        let v: serde_json::Value = serde_json::from_str(text).context("not valid JSON")?;
        let shape = shape_from_value(&v)?;
        shape.validate()?;
        Ok(shape)
    }

    /// A shape from an SVG drawing: every `polyline`, `polygon` (closed), `line`
    /// and `path` made of straight segments (`M L H V Z`, either case). SVG's y
    /// runs down, so it is turned up, and the drawing is centred on the middle
    /// of its outline. Curves in paths are refused: draw them as polylines.
    pub fn from_svg(text: &str) -> Result<CustomShape> {
        let mut lines = Vec::new();
        for (tag, attrs) in svg_elements(text) {
            match tag.as_str() {
                "polyline" | "polygon" => {
                    let nums = numbers(attr(&attrs, "points").unwrap_or_default())?;
                    ensure!(
                        nums.len() % 2 == 0,
                        "a {tag} has an odd number of coordinates"
                    );
                    let points: Vec<[f32; 2]> = nums.chunks(2).map(|c| [c[0], c[1]]).collect();
                    if points.len() >= 2 {
                        lines.push(ShapeLine {
                            points,
                            closed: tag == "polygon",
                        });
                    }
                }
                "line" => {
                    let n = |k: &str| -> Result<f32> {
                        attr(&attrs, k)
                            .unwrap_or("0")
                            .trim()
                            .parse()
                            .with_context(|| format!("a line's {k} isn't a number"))
                    };
                    lines.push(ShapeLine {
                        points: vec![[n("x1")?, n("y1")?], [n("x2")?, n("y2")?]],
                        closed: false,
                    });
                }
                "path" => lines.extend(path_lines(attr(&attrs, "d").unwrap_or_default())?),
                _ => {}
            }
        }
        ensure!(
            !lines.is_empty(),
            "the SVG has no polyline, polygon, line or straight path to use"
        );
        // Centre on the outline's middle and turn y up.
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in lines.iter().flat_map(|l| &l.points) {
            lo = lo.min(Vec2::from(*p));
            hi = hi.max(Vec2::from(*p));
        }
        let mid = (lo + hi) * 0.5;
        for p in lines.iter_mut().flat_map(|l| &mut l.points) {
            *p = [p[0] - mid.x, mid.y - p[1]];
        }
        let shape = CustomShape {
            lines,
            mirror_x: false,
        };
        shape.validate()?;
        Ok(shape)
    }

    /// The shape as an SVG drawing (y down), for other programs.
    pub fn to_svg(&self) -> String {
        let mut body = String::new();
        for (pts, closed) in self.drawn_lines() {
            let list: Vec<String> = pts
                .iter()
                .map(|p| format!("{:.4},{:.4}", p.x, -p.y))
                .collect();
            let tag = if closed { "polygon" } else { "polyline" };
            body.push_str(&format!(
                "  <{tag} points=\"{}\" fill=\"none\" stroke=\"black\" stroke-width=\"0.02\"/>\n",
                list.join(" ")
            ));
        }
        let r = self.reach().max(1e-6) * 1.1;
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\">\n{body}</svg>\n",
            -r,
            -r,
            2.0 * r,
            2.0 * r
        )
    }
}

fn shape_from_value(v: &serde_json::Value) -> Result<CustomShape> {
    let point = |p: &serde_json::Value| -> Result<[f32; 2]> {
        let a = p.as_array().context("a point must be a list of numbers")?;
        ensure!(a.len() >= 2, "a point needs two numbers");
        let n = |x: &serde_json::Value| -> Result<f32> {
            Ok(x.as_f64().context("a point's coordinate isn't a number")? as f32)
        };
        Ok([n(&a[0])?, n(&a[1])?])
    };
    let line = |v: &serde_json::Value| -> Result<ShapeLine> {
        let (pts, closed) = match v {
            serde_json::Value::Array(a) => (a, false),
            serde_json::Value::Object(o) => (
                o.get("points")
                    .and_then(|p| p.as_array())
                    .context("a line needs \"points\"")?,
                o.get("closed").and_then(|c| c.as_bool()).unwrap_or(false),
            ),
            _ => bail!("a line must be a list of points or an object with \"points\""),
        };
        Ok(ShapeLine {
            points: pts.iter().map(point).collect::<Result<_>>()?,
            closed,
        })
    };
    let mirror = v.get("mirror_x").and_then(|m| m.as_bool()).unwrap_or(false);
    match v {
        serde_json::Value::Array(a) if a.first().is_some_and(|p| p.is_number()) => {
            bail!("expected a list of [x, y] points")
        }
        serde_json::Value::Array(_) => Ok(CustomShape {
            lines: vec![line(v)?],
            mirror_x: false,
        }),
        serde_json::Value::Object(o) => {
            if let Some(lines) = o.get("lines").and_then(|l| l.as_array()) {
                Ok(CustomShape {
                    lines: lines.iter().map(line).collect::<Result<_>>()?,
                    mirror_x: mirror,
                })
            } else if o.contains_key("points") {
                Ok(CustomShape {
                    lines: vec![line(v)?],
                    mirror_x: mirror,
                })
            } else if let Some(shapes) = o.get("shapes").and_then(|s| s.as_object()) {
                // A whole library: its first shape.
                let (_, first) = shapes.iter().next().context("the library has no shapes")?;
                shape_from_value(first)
            } else {
                bail!("expected \"lines\" or \"points\"")
            }
        }
        _ => bail!("expected a shape object or a list of points"),
    }
}

/// Tags and their attributes, in order, from an SVG (or any XML) text. Enough
/// for drawings: no entities, no CDATA.
fn svg_elements(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        if rest.starts_with('!') || rest.starts_with('?') || rest.starts_with('/') {
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let inner = rest[..end].trim_end_matches('/');
        rest = &rest[end + 1..];
        let tag_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let tag = inner[..tag_end].to_ascii_lowercase();
        // Namespaced tags (`svg:polyline`) count by their local name.
        let tag = tag.rsplit(':').next().unwrap_or("").to_string();
        let mut attrs = Vec::new();
        let mut a = &inner[tag_end..];
        while let Some(eq) = a.find('=') {
            let key = a[..eq].trim().to_ascii_lowercase();
            let after = a[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                break;
            };
            let Some(close) = after[1..].find(q) else {
                break;
            };
            attrs.push((key, after[1..1 + close].to_string()));
            a = &after[close + 2..];
        }
        out.push((tag, attrs));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The numbers in an SVG list (separated by spaces, commas or signs).
fn numbers(text: &str) -> Result<Vec<f32>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f32>| -> Result<()> {
        if !cur.is_empty() {
            out.push(
                cur.parse()
                    .with_context(|| format!("'{cur}' isn't a number"))?,
            );
            cur.clear();
        }
        Ok(())
    };
    let mut prev = ' ';
    for ch in text.chars() {
        let starts_new = (ch == '-' || ch == '+') && !matches!(prev, 'e' | 'E')
            || ch == '.' && cur.contains('.') && !cur.contains(['e', 'E']);
        if ch.is_whitespace() || ch == ',' {
            flush(&mut cur, &mut out)?;
        } else if starts_new {
            flush(&mut cur, &mut out)?;
            cur.push(ch);
        } else {
            cur.push(ch);
        }
        prev = ch;
    }
    flush(&mut cur, &mut out)?;
    Ok(out)
}

/// The lines of an SVG path made of straight segments.
fn path_lines(d: &str) -> Result<Vec<ShapeLine>> {
    // Split into commands and their numbers.
    let mut cmds: Vec<(char, Vec<f32>)> = Vec::new();
    let mut start = 0;
    let letters: Vec<(usize, char)> = d
        .char_indices()
        .filter(|(_, c)| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E'))
        .collect();
    for (k, &(i, c)) in letters.iter().enumerate() {
        ensure!(
            d[start..i].trim().is_empty() || !cmds.is_empty(),
            "a path must start with a move (M)"
        );
        let end = letters.get(k + 1).map_or(d.len(), |&(j, _)| j);
        cmds.push((c, numbers(&d[i + 1..end])?));
        start = end;
    }
    let mut lines = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let mut at = Vec2::ZERO;
    let mut begin = Vec2::ZERO;
    let finish = |cur: &mut Vec<[f32; 2]>, lines: &mut Vec<ShapeLine>, closed: bool| {
        if cur.len() >= 2 {
            lines.push(ShapeLine {
                points: std::mem::take(cur),
                closed,
            });
        }
        cur.clear();
    };
    for (c, nums) in cmds {
        let rel = c.is_ascii_lowercase();
        match c.to_ascii_uppercase() {
            'M' | 'L' => {
                ensure!(
                    nums.len() % 2 == 0 && !nums.is_empty(),
                    "a path's {c} needs pairs of numbers"
                );
                for (k, pair) in nums.chunks(2).enumerate() {
                    let p = Vec2::new(pair[0], pair[1]);
                    at = if rel { at + p } else { p };
                    if c.eq_ignore_ascii_case(&'M') && k == 0 {
                        finish(&mut cur, &mut lines, false);
                        begin = at;
                    }
                    cur.push(at.to_array());
                }
            }
            'H' | 'V' => {
                for v in nums {
                    if c.eq_ignore_ascii_case(&'H') {
                        at.x = if rel { at.x + v } else { v };
                    } else {
                        at.y = if rel { at.y + v } else { v };
                    }
                    cur.push(at.to_array());
                }
            }
            'Z' => {
                if cur.last() == Some(&begin.to_array()) && cur.len() > 2 {
                    cur.pop();
                }
                finish(&mut cur, &mut lines, true);
                at = begin;
            }
            other => bail!(
                "the path uses '{other}', a curve or arc; only straight segments (M, L, H, V, Z) can be read"
            ),
        }
    }
    finish(&mut cur, &mut lines, false);
    Ok(lines)
}

/// Points evenly spaced along `strokes`, seen face on (across, up).
fn samples(strokes: &Strokes) -> Vec<Vec2> {
    const SPACING: f32 = 0.05;
    let mut out = Vec::new();
    for line in strokes {
        for w in line.windows(2) {
            let (a, b) = (Vec2::new(w[0].x, w[0].z), Vec2::new(w[1].x, w[1].z));
            let n = ((a.distance(b) / SPACING).ceil() as usize).max(1);
            out.extend((0..n).map(|i| a.lerp(b, i as f32 / n as f32)));
        }
        if let Some(p) = line.last() {
            out.push(Vec2::new(p.x, p.z));
        }
    }
    out
}

/// How unlike two sampled outlines are: the mean distance from each point to
/// the other outline, both ways.
fn likeness(a: &[Vec2], b: &[Vec2]) -> f32 {
    let one_way = |from: &[Vec2], to: &[Vec2]| {
        from.iter()
            .map(|p| {
                to.iter()
                    .map(|q| p.distance_squared(*q))
                    .fold(f32::MAX, f32::min)
            })
            .map(f32::sqrt)
            .sum::<f32>()
            / from.len().max(1) as f32
    };
    one_way(a, b) + one_way(b, a)
}

/// A shape name made safe: letters, digits, `_`, `-` and spaces, trimmed.
pub fn clean_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Check a name for a custom shape.
pub fn check_name(name: &str) -> Result<()> {
    ensure!(!name.trim().is_empty(), "a shape needs a name");
    ensure!(
        clean_name(name) == name,
        "'{name}' can only have letters, digits, spaces, '_' and '-'"
    );
    ensure!(
        name.len() <= 64,
        "'{name}' is too long (64 characters at most)"
    );
    Ok(())
}

// --------------------------------------------------------------------------- library

/// The user's own shapes, kept between characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeLibrary {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub shapes: BTreeMap<String, CustomShape>,
}

impl Default for ShapeLibrary {
    fn default() -> Self {
        ShapeLibrary {
            format: LIBRARY_FORMAT.into(),
            version: LIBRARY_VERSION,
            shapes: BTreeMap::new(),
        }
    }
}

impl ShapeLibrary {
    /// Read a library file; a missing file is an empty library.
    pub fn load(path: &Path) -> Result<ShapeLibrary> {
        if !path.exists() {
            return Ok(ShapeLibrary::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let lib: ShapeLibrary =
            serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            lib.format == LIBRARY_FORMAT,
            "{} is not a hive-rig shape library (format is '{}')",
            path.display(),
            lib.format
        );
        ensure!(
            lib.version <= LIBRARY_VERSION,
            "{} was written by a newer hive-rig (shape library version {})",
            path.display(),
            lib.version
        );
        Ok(lib)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Add or replace a shape.
    pub fn insert(&mut self, name: &str, shape: CustomShape) -> Result<()> {
        check_name(name)?;
        shape.validate()?;
        self.shapes.insert(name.to_string(), shape);
        Ok(())
    }
}

/// The user's shape library file: `%APPDATA%\HiveRig\shapes.json` on Windows,
/// otherwise `$XDG_CONFIG_HOME/hive-rig/shapes.json` or
/// `~/.config/hive-rig/shapes.json`. `HIVE_RIG_SHAPES` names another file.
pub fn library_path() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(p) = var("HIVE_RIG_SHAPES") {
        return Some(p);
    }
    if cfg!(windows) {
        return var("APPDATA").map(|d| d.join("HiveRig").join("shapes.json"));
    }
    var("XDG_CONFIG_HOME")
        .or_else(|| var("HOME").map(|h| h.join(".config")))
        .map(|d| d.join("hive-rig").join("shapes.json"))
}

/// Clamp a line width into [`LINE_WIDTH_RANGE`].
pub fn clamp_width(w: f32) -> f32 {
    w.clamp(LINE_WIDTH_RANGE.0, LINE_WIDTH_RANGE.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_heart() -> CustomShape {
        // From the bottom tip up the right side and back to the dip at the top.
        CustomShape::line(
            vec![[0.0, -1.0], [0.8, 0.0], [0.9, 0.6], [0.5, 0.9], [0.0, 0.5]],
            false,
            true,
        )
    }

    #[test]
    fn mirrored_half_outline_closes_into_one_loop() {
        let s = half_heart();
        s.validate().unwrap();
        let lines = s.drawn_lines();
        assert_eq!(lines.len(), 1);
        let (pts, closed) = &lines[0];
        assert!(closed);
        // Five points, three of them mirrored (the two on the axis aren't doubled).
        assert_eq!(pts.len(), 8);
        assert!(pts.iter().any(|p| (p.x + 0.9).abs() < 1e-6));
        let strokes = s.strokes();
        assert_eq!(strokes[0].first(), strokes[0].last());
        // Fitted to unit size: the bottom tip is at -1 up.
        let reach = strokes[0]
            .iter()
            .map(|p| p.abs().max_element())
            .fold(0.0, f32::max);
        assert!((reach - 1.0).abs() < 1e-6);
        assert!(strokes.iter().flatten().all(|p| p.y == 0.0));
    }

    #[test]
    fn mirror_joins_or_doubles_open_lines() {
        // Ends on the axis once: one longer open line through the axis.
        let s = CustomShape::line(vec![[0.0, 0.0], [1.0, 1.0]], false, true);
        let l = s.drawn_lines();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0.len(), 3);
        assert!(!l[0].1);
        // Off the axis: a mirrored copy.
        let s = CustomShape::line(vec![[0.5, 0.0], [1.0, 1.0]], false, true);
        let l = s.drawn_lines();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].0[0], Vec2::new(-0.5, 0.0));
        // On the axis: left alone.
        let s = CustomShape::line(vec![[0.0, -1.0], [0.0, 1.0]], false, true);
        assert_eq!(s.drawn_lines().len(), 1);
        // Mirror off: as drawn, closed lines repeat their first point.
        let s = CustomShape::line(vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]], true, false);
        let st = s.strokes();
        assert_eq!(st[0].len(), 4);
        assert_eq!(st[0][1], Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn bad_shapes_are_refused() {
        assert!(
            CustomShape {
                lines: vec![],
                mirror_x: false
            }
            .validate()
            .is_err()
        );
        assert!(
            CustomShape::line(vec![[0.0, 0.0]], false, false)
                .validate()
                .is_err()
        );
        assert!(
            CustomShape::line(vec![[0.0, 0.0], [0.0, 0.0]], false, false)
                .validate()
                .is_err()
        );
        assert!(
            CustomShape::line(vec![[0.0, 0.0], [f32::NAN, 1.0]], false, false)
                .validate()
                .is_err()
        );
        assert!(check_name("my shape-2").is_ok());
        assert!(check_name("").is_err() && check_name("a/b").is_err());
        assert_eq!(clean_name(" a/b "), "a_b");
    }

    #[test]
    fn look_alikes_find_the_built_in_shape() {
        // A square drawn by hand looks like the square, a hexagon like the hexagon.
        let square = CustomShape::line(
            vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
            true,
            false,
        );
        assert_eq!(square.look_alike(), Shape::Square);
        let hex: Vec<[f32; 2]> = (0..6)
            .map(|i| {
                let a = (90.0 + 60.0 * i as f32).to_radians();
                [a.cos(), a.sin()]
            })
            .collect();
        assert_eq!(
            CustomShape::line(hex, true, false).look_alike(),
            Shape::Hexagon
        );
        let cross = CustomShape {
            lines: vec![
                ShapeLine {
                    points: vec![[-1.0, 0.0], [1.0, 0.0]],
                    closed: false,
                },
                ShapeLine {
                    points: vec![[0.0, -1.0], [0.0, 1.0]],
                    closed: false,
                },
            ],
            mirror_x: false,
        };
        assert_eq!(cross.look_alike(), Shape::Cross);
    }

    #[test]
    fn reads_json_in_every_form() {
        let full = r#"{"lines":[{"points":[[0,1],[1,0],[0,-1]],"closed":true}],"mirror_x":true}"#;
        let s = CustomShape::from_json(full).unwrap();
        assert!(s.mirror_x && s.lines[0].closed);
        let one = CustomShape::from_json(r#"{"points":[[0,0,5],[1,1,5]]}"#).unwrap();
        assert_eq!(one.lines[0].points, vec![[0.0, 0.0], [1.0, 1.0]]);
        let bare = CustomShape::from_json("[[0,0],[1,2],[3,1]]").unwrap();
        assert!(!bare.lines[0].closed && bare.lines[0].points.len() == 3);
        let lib = r#"{"format":"hive-rig-shapes","version":1,"shapes":{"tri":{"lines":[{"points":[[0,1],[1,-1],[-1,-1]],"closed":true}]}}}"#;
        assert_eq!(
            CustomShape::from_json(lib).unwrap().lines[0].points.len(),
            3
        );
        assert!(CustomShape::from_json("[1,2]").is_err());
        assert!(CustomShape::from_json(r#"{"nope":1}"#).is_err());
        // The shape round-trips through its own serde form.
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<CustomShape>(&text).unwrap(), s);
    }

    #[test]
    fn reads_svg_polylines_polygons_lines_and_paths() {
        let svg = r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <!-- a comment -->
  <polygon points="50,0 100,100 0,100" fill="none"/>
  <polyline points='10 50, 90 50'/>
  <line x1="50" y1="0" x2="50" y2="100"/>
  <path d="M10,10 l10,0 v10 h-10 z M 0 0 L-5-5"/>
</svg>"#;
        let s = CustomShape::from_svg(svg).unwrap();
        assert_eq!(s.lines.len(), 5);
        // The triangle points up once y is turned, centred on the drawing.
        let tri = &s.lines[0];
        assert!(tri.closed);
        let top = tri.points.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert_eq!(tri.points[0][1], top);
        // The square path closes and doesn't repeat its first point.
        let sq = &s.lines[3];
        assert!(sq.closed && sq.points.len() == 4);
        // Exponents and run-together signs.
        assert_eq!(numbers("1e1-2.5.5,3").unwrap(), vec![10.0, -2.5, 0.5, 3.0]);
        assert!(CustomShape::from_svg(r#"<svg><path d="M0 0 C 1 1 2 2 3 3"/></svg>"#).is_err());
        assert!(CustomShape::from_svg("<svg><rect/></svg>").is_err());
        // SVG out and back keeps the lines.
        let back = CustomShape::from_svg(&half_heart().to_svg()).unwrap();
        assert_eq!(back.lines.len(), 1);
        assert!(back.lines[0].closed && back.lines[0].points.len() == 8);
    }

    #[test]
    fn library_saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("hive-rig-shapes-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let path = dir.join("shapes.json");
        assert!(ShapeLibrary::load(&path).unwrap().shapes.is_empty());
        let mut lib = ShapeLibrary::default();
        lib.insert("heart", half_heart()).unwrap();
        assert!(lib.insert("bad/name", half_heart()).is_err());
        lib.save(&path).unwrap();
        let back = ShapeLibrary::load(&path).unwrap();
        assert_eq!(back.shapes["heart"], half_heart());
        std::fs::write(&path, r#"{"format":"other","version":1}"#).unwrap();
        assert!(ShapeLibrary::load(&path).is_err());
        // Import names the shape after the file.
        let svg = dir.join("Wing Tip.svg");
        std::fs::write(&svg, half_heart().to_svg()).unwrap();
        let (name, _) = CustomShape::import(&svg).unwrap();
        assert_eq!(name, "Wing Tip");
        std::fs::remove_dir_all(&dir).ok();
    }
}
