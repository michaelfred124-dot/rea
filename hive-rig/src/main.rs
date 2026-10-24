use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use hive_rig::components::{self, AddOptions, Guides, Kind, Preset};
use hive_rig::import::Character;
use hive_rig::orient::{self, Axis, Choice, Detection, Orientation};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::Side;
use hive_rig::skin::{Engine, SkinLimits, SkinOptions};
use hive_rig::template::{self, Template};
use hive_rig::{
    Rig, RigOptions, Target, build_rig, checks, demo, export, import, mesh::Mesh, naming, rig,
};

#[derive(Parser)]
#[command(
    name = "hive-rig",
    version,
    about = "Automatic humanoid rigger with controls for Blender, Maya and Unreal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rig a character (.obj, .gltf, .glb or binary .fbx; a biped in T- or A-pose, or an
    /// animal on four legs). Its up axis and facing are detected (or given with
    /// --up and --forward).
    Rig {
        input: PathBuf,
        /// Output directory.
        #[arg(short, long, default_value = "hive_out")]
        out: PathBuf,
        /// Rig name (defaults to the input file name).
        #[arg(short, long)]
        name: Option<String>,
        /// Build from this guides file instead of detecting the body.
        #[arg(short, long)]
        guides: Option<PathBuf>,
        /// Detect the body even when the file has a skeleton (by default its
        /// joints become the guides).
        #[arg(long)]
        ignore_joints: bool,
        #[command(flatten)]
        orient: OrientFlags,
        #[command(flatten)]
        common: Common,
    },
    /// Rig the built-in test character to try the pipeline without an asset.
    Demo {
        #[arg(short, long, default_value = "hive_demo")]
        out: PathBuf,
        /// Lower the demo character's arms by this many degrees (45 = A-pose).
        #[arg(long, default_value_t = 0.0, conflicts_with_all = ["quadruped", "creature"])]
        arm_drop: f32,
        /// Rig the built-in dog instead (the same as `--creature dog`).
        #[arg(long, conflicts_with = "creature")]
        quadruped: bool,
        /// Rig a built-in creature instead, written as `hive_<creature>`: a dog
        /// (four legs with three-segment hind legs), a bird (wings, two legs
        /// with hocks, tail), a spider (eight FK legs), a snake (spline spine
        /// and tail with a head), a horse (long legs, spring hocks) or a
        /// dinosaur (a theropod: three-segment legs, short arms, spline tail).
        #[arg(long, value_enum)]
        creature: Option<demo::Creature>,
        /// Make the arms and legs (and quad legs) bendy: bend controls and an
        /// elbow or knee pin.
        #[arg(long)]
        bendy: bool,
        /// Default soft IK (0 to 1) on the arms and legs (and quad legs).
        #[arg(long, default_value_t = 0.0)]
        soft: f32,
        /// Default hock spring (0 to 1) on the quad legs: at 1 the hock swings
        /// with the thigh as the paw moves. The horse starts at 1, the others at 0.
        #[arg(long)]
        spring: Option<f32>,
        #[command(flatten)]
        common: Common,
    },
    /// Rebuild a rig from its (edited) guides, reusing the mesh in its .hive.json.
    Rebuild {
        /// The `<name>.hive.json` of the rig to rebuild.
        rig: PathBuf,
        /// Guides to build from (default: `<name>.guides.json` next to the rig).
        #[arg(short, long)]
        guides: Option<PathBuf>,
        /// Output directory (default: the rig's directory).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Keep the weights edited by hand (painted, mirrored, smoothed or copied)
        /// in the old rig, except where the new guides add a bone that claims
        /// the vertex.
        #[arg(long)]
        keep_weights: bool,
        #[command(flatten)]
        common: Common,
    },
    /// Skin weight tools for a rig that has been built.
    Weights {
        #[command(subcommand)]
        action: WeightsCommand,
    },
    /// Blendshapes on a built rig: sculpt corrective shapes in a pose, drive
    /// them from a joint's rotation, mirror, split, combine and set weights.
    /// Writes the rig again (its .glb, .hive.json and the scripts and .fbx
    /// already next to it).
    Shape {
        /// The `<name>.hive.json` of the rig.
        rig: PathBuf,
        #[command(subcommand)]
        action: ShapeCommand,
    },
    /// Add a premade component to a guides file.
    Add {
        guides: PathBuf,
        preset: Preset,
        /// Which hands get fingers (default: both).
        #[arg(long)]
        side: Option<SideArg>,
        /// Bone to attach a tail or chain to.
        #[arg(long)]
        parent: Option<String>,
        /// Bones in a tail or chain.
        #[arg(long)]
        joints: Option<usize>,
        /// Component id for a tail or chain.
        #[arg(long)]
        id: Option<String>,
        /// Curve controls on a tentacle (ends included, default 4).
        #[arg(long)]
        controls: Option<usize>,
    },
    /// Change the spine: its joint count, and whether a curve between the hips
    /// and chest controls drives it (a spline spine) instead of FK.
    Spine {
        guides: PathBuf,
        /// Spine joints (1 to 16).
        #[arg(long)]
        joints: Option<usize>,
        /// Drive the spine with a curve.
        #[arg(long, action = clap::ArgAction::Set)]
        spline: Option<bool>,
        /// Curve controls of a spline spine (ends included, default 3).
        #[arg(long)]
        controls: Option<usize>,
    },
    /// Hang a component from another bone (its guides stay where they are).
    Attach {
        guides: PathBuf,
        /// Component id.
        id: String,
        /// Bone to hang it from.
        bone: String,
    },
    /// Import the premade components of another guides file, hung from a bone.
    Import {
        guides: PathBuf,
        /// The guides file to take components from.
        from: PathBuf,
        /// Bone to hang the imported components from.
        #[arg(long)]
        parent: String,
    },
    /// Remove a component (and everything attached to it) from a guides file.
    Remove { guides: PathBuf, id: String },
    /// List the components in a guides file.
    Components { guides: PathBuf },
    /// Look for likely mistakes before building: guides on the wrong side,
    /// limbs bending the wrong way or not at all, joints outside the body or on
    /// top of each other, lopsided twins and odd parents. Exits non-zero only
    /// when the guides can't build.
    Check {
        /// A `.guides.json`, or a `.hive.json` (its mesh is checked against,
        /// and the guides next to it, which a rebuild would use).
        file: PathBuf,
        /// A character mesh (.obj, .gltf, .glb, .fbx) to check the guides against.
        #[arg(long)]
        mesh: Option<PathBuf>,
        /// Apply the fixes that are mechanical and save the guides file.
        #[arg(long)]
        fix: bool,
        #[command(flatten)]
        orient: OrientFlags,
    },
    /// Save a layout as a template, load one onto a layout, or list them.
    Template {
        #[command(subcommand)]
        action: TemplateCommand,
    },
    /// Show or set how a guides file names joints and controls on export.
    Naming {
        guides: PathBuf,
        /// game (Unreal mannequin), prefix (L_arm_jnt) or suffix (arm_L_JNT).
        preset: Option<String>,
        /// Joint pattern using {name}, {side} and {desc}.
        #[arg(long)]
        joint: Option<String>,
        /// Control pattern using {name}, {side} and {desc}.
        #[arg(long)]
        control: Option<String>,
        #[arg(long)]
        left: Option<String>,
        #[arg(long)]
        right: Option<String>,
        /// Centre side token (empty to leave it out).
        #[arg(long)]
        center: Option<String>,
    },
}

#[derive(Subcommand)]
enum TemplateCommand {
    /// Save the whole layout, or only some premade parts, as `<name>.template.json`.
    Save {
        guides: PathBuf,
        /// The template file (`<name>.template.json`), or a folder to save it in.
        out: PathBuf,
        /// Only these components (premade parts such as fingers, a tail or eyes).
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
        /// Template name (default: the file name).
        #[arg(long)]
        name: Option<String>,
        /// A line describing the template.
        #[arg(long)]
        description: Option<String>,
    },
    /// Load a template onto a guides file: a whole layout replaces the guides
    /// (scaled to this body), parts are added (ids in use get a number).
    Apply {
        guides: PathBuf,
        /// A template file, or the name of a built-in or saved template.
        template: String,
        /// Bone to hang parts from whose parent isn't in the template.
        #[arg(long)]
        parent: Option<String>,
    },
    /// List the built-in templates and the ones in the template folders.
    List,
}

#[derive(Subcommand)]
enum ShapeCommand {
    /// List the shapes, their weights and what drives them.
    List,
    /// Move vertices in a pose and keep the change as (part of) a shape: the
    /// offsets are worked back through the skin, so the shape puts the
    /// vertices there in that pose.
    Sculpt {
        name: String,
        /// The pose to sculpt in: BONE=DEGREES, or BONE=DEGREES@DIRECTION for
        /// the way the bone's tip swings (up, down, forward, back, left, right
        /// or x,y,z). A limb's middle bone (lowerarm_l, calf_r) bends the way
        /// its IK bends without a direction. Leave out to sculpt at rest.
        #[arg(long)]
        pose: Option<String>,
        /// Brush centre (x,y,z in metres, in the posed mesh).
        #[arg(long, requires_all = ["radius", "offset"], conflicts_with = "from")]
        at: Option<String>,
        /// Brush radius in metres; vertices fade out toward the edge.
        #[arg(long)]
        radius: Option<f32>,
        /// How far the brush moves the vertices at its centre (x,y,z metres).
        #[arg(long)]
        offset: Option<String>,
        /// A sculpted copy of the mesh (same vertices, .obj/.gltf/.glb), posed
        /// as `--pose` says (or at rest).
        #[arg(long)]
        from: Option<PathBuf>,
        /// Also drive the shape from the pose: off at rest, fully on at the
        /// pose's angle.
        #[arg(long, requires = "pose")]
        drive: bool,
    },
    /// Drive a shape from a bone's rotation (a corrective shape): its weight
    /// goes from 0 at `--start` degrees to 1 at `--end` degrees of turn.
    Drive {
        name: String,
        #[arg(long)]
        bone: String,
        /// The way the bone's tip swings to turn the shape on (up, down,
        /// forward, back, left, right or x,y,z). Default for a limb's middle
        /// bone: the way it bends.
        #[arg(long, conflicts_with = "axis")]
        toward: Option<String>,
        /// The axis the turn is read about (x,y,z, model space, right-handed).
        #[arg(long)]
        axis: Option<String>,
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        start: f32,
        #[arg(long, default_value_t = 90.0, allow_hyphen_values = true)]
        end: f32,
    },
    /// Stop driving a shape; it keeps its own weight.
    Undrive {
        name: String,
    },
    /// Mirror a shape onto the other side (smile_l to smile_r), with its driver.
    Mirror {
        name: String,
        /// Name of the mirrored shape (default: the side token swapped).
        #[arg(long = "as")]
        new: Option<String>,
    },
    /// Split a shape into left and right halves (`<name>_l`, `<name>_r`).
    Split {
        name: String,
        /// Width of the blend across the centre line, in metres (default 2% of
        /// the body height).
        #[arg(long)]
        width: Option<f32>,
        /// Names of the halves: LEFT,RIGHT.
        #[arg(long, value_delimiter = ',')]
        names: Option<Vec<String>>,
    },
    /// Add shapes together as a new shape.
    Combine {
        /// The shapes to add, comma separated.
        #[arg(value_delimiter = ',')]
        names: Vec<String>,
        #[arg(long = "as")]
        new: String,
    },
    /// Set the weight a shape sits at (0 off, 1 fully on).
    Weight {
        name: String,
        #[arg(allow_hyphen_values = true)]
        value: f32,
    },
    Remove {
        name: String,
    },
    Rename {
        name: String,
        new: String,
    },
}

#[derive(Subcommand)]
enum WeightsCommand {
    /// Copy one rig's weights onto another character's rig with the same or a
    /// similar skeleton, in any pose or size, and write the target rig again.
    Copy {
        /// The `<name>.hive.json` to copy from.
        from: PathBuf,
        /// The `<name>.hive.json` to copy onto.
        to: PathBuf,
        /// Output directory (default: the target rig's directory).
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        skin: SkinFlags,
        #[command(flatten)]
        output: Outputs,
    },
    /// Prune small weights and cap the influences per vertex of a built rig,
    /// keeping its painted weights. The limits are stored with the guides, so
    /// rebuilds keep them.
    Limit {
        /// The `<name>.hive.json` of the rig.
        rig: PathBuf,
        /// Output directory (default: the rig's directory).
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        skin: SkinFlags,
        #[command(flatten)]
        output: Outputs,
    },
    /// Make every vertex's weights add up to 1 within the rig's influence
    /// limits.
    Normalize {
        /// The `<name>.hive.json` of the rig.
        rig: PathBuf,
        #[command(flatten)]
        edit: WeightEdit,
    },
    /// Replace every vertex whose weights stand out from its neighbours'
    /// (a spike) with their average.
    FixSpikes {
        /// The `<name>.hive.json` of the rig.
        rig: PathBuf,
        /// How far a vertex must differ from its neighbours' average, from 0 to
        /// 1: 0.5 is a hard edge between two bones, 1 a vertex sharing no bone
        /// with them.
        #[arg(long, default_value_t = hive_rig::skin::SPIKE)]
        threshold: f32,
        #[command(flatten)]
        edit: WeightEdit,
    },
    /// Re-run the automatic weights on the skin of some bones (wherever they
    /// have weight now or would get it), keeping the hand edits elsewhere.
    Auto {
        /// The `<name>.hive.json` of the rig.
        rig: PathBuf,
        /// A bone whose skin gets the automatic weights again (repeatable).
        #[arg(long, required = true)]
        bone: Vec<String>,
        #[command(flatten)]
        edit: WeightEdit,
    },
}

/// What every hand edit of a rig's weights takes.
#[derive(clap::Args)]
struct WeightEdit {
    /// A bone whose weights stay as they are (repeatable).
    #[arg(long)]
    lock: Vec<String>,
    /// Output directory (default: the rig's directory).
    #[arg(short, long)]
    out: Option<PathBuf>,
    #[command(flatten)]
    output: Outputs,
}

impl WeightEdit {
    /// The lock mask for `rig`, refusing names that aren't its bones.
    fn locks(&self, rig: &Rig) -> Result<Vec<bool>> {
        for name in &self.lock {
            anyhow::ensure!(
                rig.skeleton.index_of(name).is_some(),
                "'{name}' is not a bone of '{}'",
                rig.name
            );
        }
        Ok(hive_rig::skin::edit::lock_mask(
            &rig.skeleton,
            self.lock.iter().map(String::as_str),
        ))
    }

    fn write(&self, rig: &Rig, path: &Path) -> Result<()> {
        let dir = self
            .out
            .clone()
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_path_buf());
        write(rig, &dir, &self.output)
    }
}

/// The source file's up axis and facing, when detection gets them wrong.
#[derive(clap::Args, Default, Clone, Copy)]
struct OrientFlags {
    /// The file's axis that points up through the character: +x, -x, +y, -y,
    /// +z or -z (default: detected from the feet, legs, head and skeleton).
    #[arg(long, value_parser = parse_axis, allow_hyphen_values = true)]
    up: Option<Axis>,
    /// The file's axis the character faces along (default: detected from the
    /// toes, or for an animal on four legs its head end).
    #[arg(long, value_parser = parse_axis, allow_hyphen_values = true)]
    forward: Option<Axis>,
}

impl OrientFlags {
    fn choice(self) -> Choice {
        Choice {
            up: self.up,
            forward: self.forward,
        }
    }
}

fn parse_axis(s: &str) -> std::result::Result<Axis, String> {
    s.parse::<Axis>().map_err(|e| e.to_string())
}

#[derive(Clone, Copy, ValueEnum)]
enum SideArg {
    Left,
    Right,
}

#[derive(clap::Args)]
struct Outputs {
    /// What to write besides the .glb: builder scripts for blender, maya and
    /// unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for
    /// game engines such as Unreal and Unity.
    #[arg(
        short,
        long,
        value_delimiter = ',',
        default_value = "blender,maya,unreal"
    )]
    targets: Vec<Target>,
}

/// Skin influence limits. Unset flags keep what the guides file says (four
/// influences and no pruning unless it was changed).
#[derive(clap::Args, Default)]
struct SkinFlags {
    /// Most bones that may influence one vertex (1 to 12). glTF writes them in
    /// sets of four.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=12))]
    max_influences: Option<u8>,
    /// Set max influences for an engine: unreal (8), unreal12 (12), unity (4)
    /// or gltf (4).
    #[arg(long, conflicts_with = "max_influences")]
    engine: Option<Engine>,
    /// Drop influences below this weight (such as 0.01) and renormalise.
    #[arg(long)]
    prune: Option<f32>,
}

impl SkinFlags {
    /// Apply the flags to the limits stored in the guides; true if they changed.
    fn apply(&self, limits: &mut SkinLimits) -> Result<bool> {
        let before = *limits;
        if let Some(n) = self.max_influences {
            limits.max_influences = n as usize;
        }
        if let Some(e) = self.engine {
            limits.max_influences = e.max_influences();
        }
        if let Some(p) = self.prune {
            limits.prune = p;
        }
        limits.validate()?;
        Ok(*limits != before)
    }
}

#[derive(clap::Args)]
struct Common {
    #[command(flatten)]
    output: Outputs,
    #[command(flatten)]
    skin: SkinFlags,
    /// Skin weight smoothing passes.
    #[arg(long, default_value_t = 3)]
    smooth: usize,
}

impl Common {
    fn options(&self) -> RigOptions {
        let mut opts = RigOptions::default();
        opts.skin.smooth_iterations = self.smooth;
        opts
    }
}

fn run(
    mesh: Mesh,
    name: &str,
    mut guides: Guides,
    out: &Path,
    common: &Common,
    orientation: Orientation,
) -> Result<()> {
    common.skin.apply(&mut guides.skin)?;
    print_warnings(&guides, &mesh);
    let mut rig = build_rig(mesh, name, guides, common.options())?;
    rig.orientation = orientation;
    write(&rig, out, &common.output)?;
    if !orientation.is_standard() {
        println!(
            "  The .glb and .fbx stand it as in the source ({orientation}); the Blender and Maya scripts build it upright, facing front."
        );
    }
    Ok(())
}

fn write(rig: &Rig, out: &Path, output: &Outputs) -> Result<()> {
    let files = export::write_all(rig, out, &output.targets)?;
    println!(
        "Rigged '{}': {} vertices, {} bones, {} controls, {} IK limbs, up to {} influences per vertex",
        rig.name,
        rig.mesh.vertex_count(),
        rig.skeleton.bones.len(),
        rig.controls.controls.len(),
        rig.controls.ik_chains.len(),
        rig.skin.width()
    );
    for f in files {
        println!("  {}", f.display());
    }
    Ok(())
}

fn detect(mesh: &Mesh) -> Guides {
    let (guides, d) = detect_guides(mesh);
    warn_undetected(&d);
    guides
}

fn warn_undetected(d: &hive_rig::landmarks::Detected) {
    for (found, what) in [(d.crotch, "legs"), (d.arms, "arms"), (d.neck, "neck")] {
        if !found {
            eprintln!("warning: could not detect the {what}; used average proportions instead");
        }
    }
}

/// Say how the character stands in its file and which layout it gets.
fn print_orientation(d: &Detection) {
    println!("Orientation: {}", d.summary());
    println!("Template: {}", d.body.template());
}

/// Guides for a character: from its joints when it has a skeleton (unless
/// `ignore_joints`), otherwise detected on the mesh; the dog layout for an
/// animal on four legs.
fn character(ch: &Character, ignore_joints: bool) -> Result<Guides> {
    let joints: &[hive_rig::joints::Joint] = if ignore_joints { &[] } else { &ch.joints };
    let (guides, d, used) = rig::character_guides(&ch.mesh, joints, ch.detection.body);
    match used {
        None => {
            warn_undetected(&d);
        }
        Some(Ok(report)) => {
            println!(
                "Read {} guides from the file's {} joints{}{}.",
                report.placed.len(),
                joints.len(),
                if report.by_shape.is_empty() {
                    String::new()
                } else {
                    format!(" ({} by the skeleton's shape)", report.by_shape.len())
                },
                if report.added.is_empty() {
                    String::new()
                } else {
                    format!(" and added {}", report.added.join(", "))
                }
            );
            if !report.unused.is_empty() {
                println!(
                    "  not used: {}",
                    report
                        .unused
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                        + if report.unused.len() > 12 {
                            ", ..."
                        } else {
                            ""
                        }
                );
            }
        }
        Some(Err(e)) => {
            eprintln!("warning: couldn't use the file's joints ({e:#}); detected the body instead");
            warn_undetected(&d);
        }
    }
    Ok(guides)
}

/// Read a guides file, warning when a newer hive-rig wrote it.
fn load_guides(path: &Path) -> Result<Guides> {
    let g = Guides::load(path)?;
    if let Some(w) = g.newer_warning() {
        eprintln!("warning: {}: {w}", path.display());
    }
    Ok(g)
}

/// Read a `.hive.json`, warning when a newer hive-rig wrote it.
fn load_rig(path: &Path) -> Result<Rig> {
    let rig = Rig::load(path)?;
    if let Some(w) = rig.newer_warning() {
        eprintln!("warning: {}: {w}", path.display());
    }
    Ok(rig)
}

/// Print the pre-build checks' warnings (a layout that can't build fails the
/// build itself, with the reason).
fn print_warnings(guides: &Guides, mesh: &Mesh) {
    let issues = checks::check(guides, Some(&mut checks::Surface::new(mesh)));
    for i in issues
        .iter()
        .filter(|i| i.severity == checks::Severity::Warning)
    {
        eprintln!("{}", i.line());
    }
}

fn check_command(file: &Path, mesh: Option<&Path>, fix: bool, orient: OrientFlags) -> Result<()> {
    let is_rig = file
        .to_string_lossy()
        .to_lowercase()
        .ends_with(".hive.json");
    let mut orientation_line = None;
    let (guides_path, mut guides, rig_mesh) = if is_rig {
        let rig = load_rig(file)?;
        orientation_line = Some(format!("{} (from the rig)", rig.orientation));
        let dir = file.parent().unwrap_or(Path::new("."));
        let next_to = dir.join(format!("{}.guides.json", rig.name));
        // A rebuild reads the guides next to the rig, so those are the ones to check.
        if next_to.is_file() {
            let g = load_guides(&next_to)?;
            (Some(next_to), g, Some(rig.mesh))
        } else {
            (None, rig.guides, Some(rig.mesh))
        }
    } else {
        (Some(file.to_path_buf()), load_guides(file)?, None)
    };
    let mut body = None;
    let mesh = match mesh {
        Some(m) => {
            let ch = import::load_character(m, orient.choice())?;
            orientation_line = Some(ch.detection.summary());
            body = Some(ch.detection.body);
            Some(ch.mesh)
        }
        None => rig_mesh,
    };
    let mut surface = mesh.as_ref().map(checks::Surface::new);
    let shown = guides_path.as_deref().unwrap_or(file);
    println!("{}: {}", shown.display(), guides.origin());
    if let Some(o) = &orientation_line {
        println!("  orientation: {o}");
    }
    println!(
        "  template: {}{}",
        orient::layout_name(&guides),
        body.map(|b| format!(" (the body looks like a {} layout)", b.template()))
            .unwrap_or_default()
    );
    if let Some(advice) = body.and_then(|b| orient::template_advice(&guides, b)) {
        println!("warning: {advice}");
    }
    if surface.is_none() {
        println!("  (no mesh given: pass --mesh or a .hive.json to check guides against the body)");
    }
    let mut issues = checks::check(&guides, surface.as_mut());
    if fix && issues.iter().any(|i| i.fix.is_some()) {
        let Some(path) = &guides_path else {
            anyhow::bail!("--fix needs a guides file to save to");
        };
        let n = checks::fix_all(&mut guides, &issues)?;
        guides.save(path)?;
        println!("Applied {n} fix(es) and saved {}", path.display());
        issues = checks::check(&guides, surface.as_mut());
    }
    for i in &issues {
        println!("{}", i.line());
    }
    let (w, e) = (checks::warnings(&issues), checks::errors(&issues));
    if issues.is_empty() {
        println!("No problems found.");
    } else {
        println!("{e} error(s), {w} warning(s)");
    }
    anyhow::ensure!(e == 0, "the guides can't build");
    Ok(())
}

/// Opened by double-clicking (no arguments) or by dropping a file on the program:
/// ask for a character, rig it, show the output folder, and wait so the console
/// window doesn't close before it can be read.
fn guided(dropped: Option<PathBuf>) -> Result<()> {
    let result = guided_rig(dropped);
    if let Err(e) = &result {
        eprintln!("\nSomething went wrong: {e:#}");
    }
    println!("\nPress Enter to close this window.");
    let _ = std::io::stdin().read_line(&mut String::new());
    result
}

fn guided_rig(dropped: Option<PathBuf>) -> Result<()> {
    println!(
        "hive-rig {}: automatic rigger for Blender, Maya and Unreal\n",
        env!("CARGO_PKG_VERSION")
    );
    let input = match dropped {
        Some(p) => Some(p),
        None => {
            println!(
                "Drag your character file (.glb, .gltf, .fbx or .obj) into this window and press Enter,"
            );
            println!("or just press Enter to rig the built-in demo character.");
            print!("> ");
            std::io::Write::flush(&mut std::io::stdout())?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            // Dropping a file into a console pastes its path, quoted when it has spaces.
            let path = line.trim().trim_matches(|c| c == '"' || c == '\'');
            (!path.is_empty()).then(|| PathBuf::from(path))
        }
    };
    let common = Common {
        output: Outputs {
            targets: Target::ALL.to_vec(),
        },
        skin: SkinFlags::default(),
        smooth: 3,
    };
    let out = match input {
        Some(input) => {
            let ch = import::load_character(&input, Choice::default())?;
            ch.mesh.validate()?;
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "character".into());
            let out = input
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("{stem}_rig"));
            println!();
            print_orientation(&ch.detection);
            let guides = character(&ch, false)?;
            let o = ch.detection.orientation;
            run(ch.mesh, &stem, guides, &out, &common, o)?;
            out
        }
        None => {
            let here = std::env::current_exe()?;
            let out = here.parent().unwrap_or(Path::new(".")).join("demo_rig");
            println!();
            let mesh = demo::humanoid(0.0);
            let guides = detect(&mesh);
            run(
                mesh,
                "hive_demo",
                guides,
                &out,
                &common,
                Orientation::STANDARD,
            )?;
            out
        }
    };
    println!(
        "\nDone. Open the _blender.py, _maya.py or _unreal.py file from this folder in that program:"
    );
    println!("  {}", out.display());
    if cfg!(windows) {
        let _ = std::process::Command::new("explorer").arg(&out).spawn();
    }
    Ok(())
}

fn template_command(action: TemplateCommand) -> Result<()> {
    match action {
        TemplateCommand::Save {
            guides,
            out,
            only,
            name,
            description,
        } => {
            let g = load_guides(&guides)?;
            let name = name.unwrap_or_else(|| {
                if out.is_dir() {
                    guides
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .trim_end_matches(".json")
                        .trim_end_matches(".guides")
                        .to_string()
                } else {
                    template::name_from_path(&out)
                }
            });
            let path = if out.is_dir() {
                template::file_for(&out, &name)
            } else {
                anyhow::ensure!(
                    out.to_string_lossy().ends_with(template::TEMPLATE_SUFFIX),
                    "name the template file <name>{}",
                    template::TEMPLATE_SUFFIX
                );
                out
            };
            let mut t = Template::from_guides(&g, only.as_deref(), &name)?;
            t.description = description.unwrap_or_default();
            t.save(&path)?;
            let what = t
                .components
                .iter()
                .map(|p| p.component.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("Saved template '{}' ({what}) to {}", t.name, path.display());
            Ok(())
        }
        TemplateCommand::Apply {
            guides,
            template: spec,
            parent,
        } => {
            let mut g = load_guides(&guides)?;
            let t = template::find(&spec)?;
            let applied = t.apply(&mut g, parent.as_deref())?;
            g.save(&guides)?;
            match t.scope {
                template::Scope::Layout => println!(
                    "Loaded the '{}' layout ({} components)",
                    t.name,
                    applied.added.len()
                ),
                template::Scope::Parts => {
                    println!("Added {} from '{}'", applied.summary(), t.name)
                }
            }
            Ok(())
        }
        TemplateCommand::List => {
            let (listed, problems) = template::list();
            for l in &listed {
                let scope = match l.template.scope {
                    template::Scope::Layout => "layout",
                    template::Scope::Parts => "parts",
                };
                let from = match &l.source {
                    template::Source::BuiltIn => "built in".to_string(),
                    template::Source::File(p) => p.display().to_string(),
                };
                println!("{:<22} {scope:<7} {from}", l.template.name);
            }
            for p in problems {
                eprintln!("warning: {p}");
            }
            let folders: Vec<String> = template::folders()
                .iter()
                .map(|d| d.display().to_string())
                .collect();
            println!("Template folders: {}", folders.join(", "));
            Ok(())
        }
    }
}

/// The outputs next to a rig besides the .glb and JSON: the scripts and .fbx
/// that are already there, so editing shapes refreshes them.
fn existing_targets(dir: &Path, name: &str) -> Vec<Target> {
    Target::ALL
        .into_iter()
        .filter(|t| {
            let file = match t {
                Target::Fbx => format!("{name}.fbx"),
                _ => format!("{name}_{}.py", t.key()),
            };
            dir.join(file).is_file()
        })
        .collect()
}

fn vector(text: &str) -> Result<glam::Vec3> {
    let n: Vec<f32> = text
        .split(',')
        .map(|x| x.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("'{text}' isn't x,y,z"))?;
    anyhow::ensure!(n.len() == 3, "'{text}' needs three numbers");
    Ok(glam::Vec3::new(n[0], n[1], n[2]))
}

fn shape_command(path: &Path, action: ShapeCommand) -> Result<()> {
    use hive_rig::shapes;
    let mut rig = load_rig(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let done = match action {
        ShapeCommand::List => {
            if rig.mesh.shapes.is_empty() {
                println!("{} has no shapes.", rig.name);
            }
            for s in &rig.mesh.shapes {
                let what = match &s.driver {
                    Some(d) => format!(
                        "driven by {} turning {:.0} to {:.0} degrees about ({:.2}, {:.2}, {:.2})",
                        d.bone, d.start, d.end, d.axis.x, d.axis.y, d.axis.z
                    ),
                    None => format!("weight {:.2}", s.weight),
                };
                println!("{:<24} {:>6} vertices  {what}", s.name, s.vertices.len());
            }
            return Ok(());
        }
        ShapeCommand::Sculpt {
            name,
            pose,
            at,
            radius,
            offset,
            from,
            drive,
        } => {
            let pose = pose.map(|p| shapes::parse_pose(&rig, &p)).transpose()?;
            let locals = pose.as_ref().map_or_else(
                || vec![glam::Quat::IDENTITY; rig.skeleton.bones.len()],
                |p| p.locals(&rig),
            );
            let moves = match (at, from) {
                (Some(at), None) => {
                    let posed = shapes::posed_mesh(&rig, &locals);
                    shapes::brush_moves(
                        &posed,
                        vector(&at)?,
                        radius.unwrap_or(0.05),
                        vector(&offset.unwrap_or_default())?,
                    )
                }
                (None, Some(file)) => {
                    // The sculpt is in the source file's frame, like the source.
                    let mut target = import::load_mesh(&file)?;
                    orient::turn_mesh(&mut target, rig.orientation.to_standard());
                    anyhow::ensure!(
                        target.vertex_count() == rig.mesh.vertex_count(),
                        "{} has {} vertices; the rig's mesh has {}",
                        file.display(),
                        target.vertex_count(),
                        rig.mesh.vertex_count()
                    );
                    let posed = shapes::posed_mesh(&rig, &locals);
                    target
                        .positions
                        .iter()
                        .enumerate()
                        .filter(|(v, p)| p.distance(posed[*v]) > shapes::EPSILON)
                        .map(|(v, p)| (v, *p))
                        .collect()
                }
                _ => anyhow::bail!("say what to move: --at, --radius and --offset, or --from"),
            };
            let moved = shapes::sculpt(&mut rig, &name, &locals, &moves)?;
            if drive && let Some(p) = &pose {
                let bone = rig.skeleton.bones[p.bone].name.clone();
                shapes::set_driver(
                    &mut rig,
                    &name,
                    Some(shapes::PoseReader {
                        bone,
                        axis: p.axis,
                        start: 0.0,
                        end: p.degrees,
                    }),
                )?;
            }
            format!("Sculpted {moved} vertices into '{name}'.")
        }
        ShapeCommand::Drive {
            name,
            bone,
            toward,
            axis,
            start,
            end,
        } => {
            let index = rig
                .skeleton
                .index_of(&bone)
                .ok_or_else(|| anyhow::anyhow!("the rig has no bone '{bone}'"))?;
            let axis = match axis {
                Some(a) => vector(&a)?.normalize_or_zero(),
                None => shapes::bend_axis(
                    &rig,
                    index,
                    toward.map(|t| shapes::parse_direction(&t)).transpose()?,
                )?,
            };
            shapes::set_driver(
                &mut rig,
                &name,
                Some(shapes::PoseReader {
                    bone: bone.clone(),
                    axis,
                    start,
                    end,
                }),
            )?;
            format!("'{name}' now follows {bone}: off at {start} degrees, on at {end}.")
        }
        ShapeCommand::Undrive { name } => {
            shapes::set_driver(&mut rig, &name, None)?;
            format!("'{name}' is no longer driven.")
        }
        ShapeCommand::Mirror { name, new } => {
            let made = shapes::mirror_on_rig(&mut rig, &name, new.as_deref())?;
            format!("Mirrored '{name}' as '{made}'.")
        }
        ShapeCommand::Split { name, width, names } => {
            if let Some(n) = &names {
                anyhow::ensure!(n.len() == 2, "--names takes two names: LEFT,RIGHT");
            }
            let names = names.as_ref().map(|n| [n[0].as_str(), n[1].as_str()]);
            let [l, r] = shapes::split_on_rig(&mut rig, &name, names, width)?;
            format!("Split '{name}' into '{l}' and '{r}'.")
        }
        ShapeCommand::Combine { names, new } => {
            shapes::combine_on_rig(&mut rig, &names, &new)?;
            format!("Combined {} into '{new}'.", names.join(", "))
        }
        ShapeCommand::Weight { name, value } => {
            shapes::set_weight(&mut rig, &name, value)?;
            format!("'{name}' sits at {value}.")
        }
        ShapeCommand::Remove { name } => {
            shapes::remove(&mut rig, &name)?;
            format!("Removed '{name}'.")
        }
        ShapeCommand::Rename { name, new } => {
            shapes::rename(&mut rig, &name, &new)?;
            format!("Renamed '{name}' to '{new}'.")
        }
    };
    rig.mesh.validate()?;
    let targets = existing_targets(dir, &rig.name);
    export::write_all(&rig, dir, &targets)?;
    println!("{done} Wrote {} again.", path.display());
    Ok(())
}

fn weights_command(action: WeightsCommand) -> Result<()> {
    match action {
        WeightsCommand::Copy {
            from,
            to,
            out,
            skin,
            output,
        } => {
            let source = load_rig(&from)?;
            let mut target = load_rig(&to)?;
            if skin.apply(&mut target.guides.skin)? {
                // New limits change the automatic weights the copy is posed by.
                target.skin = target.auto_weights(SkinOptions::default());
            }
            let (weights, report) = rig::copy_weights(&source, &target);
            target.skin = weights;
            println!(
                "Copied the weights of '{}' onto '{}': {} of {} bones matched.",
                source.name,
                target.name,
                report.matched,
                target.skeleton.bones.len()
            );
            if !report.by_position.is_empty() {
                println!(
                    "  matched by position (extra chains named differently): {}",
                    report.by_position.join(", ")
                );
            }
            if !report.unmatched.is_empty() {
                println!(
                    "  no match (skinned from the nearest matched bones): {}",
                    report.unmatched.join(", ")
                );
            }
            let dir = out.unwrap_or_else(|| to.parent().unwrap_or(Path::new(".")).to_path_buf());
            write(&target, &dir, &output)
        }
        WeightsCommand::Limit {
            rig: path,
            out,
            skin,
            output,
        } => {
            let mut r = load_rig(&path)?;
            skin.apply(&mut r.guides.skin)?;
            let before = r.skin.width();
            r.skin = hive_rig::skin::limit(&r.skin, r.guides.skin);
            println!(
                "Limited the weights of '{}' to {} influences per vertex (had up to {}), pruning below {}.",
                r.name, r.guides.skin.max_influences, before, r.guides.skin.prune
            );
            if r.guides.skin.max_influences > before {
                println!(
                    "  A higher limit takes effect on the next rebuild; the weights were kept."
                );
            }
            let dir = out.unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_path_buf());
            write(&r, &dir, &output)
        }
        WeightsCommand::Normalize { rig: path, edit } => {
            let mut r = load_rig(&path)?;
            let locks = edit.locks(&r)?;
            let (skin, n) = hive_rig::skin::edit::normalize(&r.skin, r.guides.skin, &locks);
            r.skin = skin;
            println!(
                "Normalized the weights of '{}': {n} vertices changed.",
                r.name
            );
            edit.write(&r, &path)
        }
        WeightsCommand::FixSpikes {
            rig: path,
            threshold,
            edit,
        } => {
            anyhow::ensure!(
                (0.0..=1.0).contains(&threshold),
                "the spike threshold must be between 0 and 1, not {threshold}"
            );
            let mut r = load_rig(&path)?;
            let locks = edit.locks(&r)?;
            let adjacency = hive_rig::skin::edit::surface_neighbours(&r.mesh);
            let (skin, n) = hive_rig::skin::edit::fix_spikes(
                &r.skin,
                &adjacency,
                threshold,
                r.guides.skin,
                &locks,
            );
            r.skin = skin;
            println!(
                "Fixed {n} spiky vertices of '{}' (standing out by more than {threshold}).",
                r.name
            );
            edit.write(&r, &path)
        }
        WeightsCommand::Auto {
            rig: path,
            bone,
            edit,
        } => {
            let mut r = load_rig(&path)?;
            let locks = edit.locks(&r)?;
            let auto = r.auto_weights(SkinOptions::default());
            let mut region = Vec::new();
            for name in &bone {
                let b = r
                    .skeleton
                    .index_of(name)
                    .ok_or_else(|| anyhow::anyhow!("'{name}' is not a bone of '{}'", r.name))?;
                region.extend(hive_rig::skin::edit::bone_region(&r.skin, &auto, b));
            }
            region.sort_unstable();
            region.dedup();
            let n =
                hive_rig::skin::edit::restore(&mut r.skin, &auto, &region, r.guides.skin, &locks);
            println!(
                "Re-ran the automatic weights on the skin of {} in '{}': {n} of {} vertices changed.",
                bone.join(", "),
                r.name,
                region.len()
            );
            edit.write(&r, &path)
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => return guided(None),
        // A character file dropped onto the program.
        [file] if Path::new(file).is_file() => return guided(Some(PathBuf::from(file))),
        _ => {}
    }
    match Cli::parse().command {
        Command::Rig {
            input,
            out,
            name,
            guides,
            ignore_joints,
            orient,
            common,
        } => {
            let ch = import::load_character(&input, orient.choice())?;
            ch.mesh.validate()?;
            print_orientation(&ch.detection);
            let name = name.unwrap_or_else(|| {
                input
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "character".into())
            });
            let guides = match guides {
                Some(path) => load_guides(&path)?,
                None => character(&ch, ignore_joints)?,
            };
            let o = ch.detection.orientation;
            run(ch.mesh, &name, guides, &out, &common, o)
        }
        Command::Demo {
            out,
            quadruped,
            creature,
            bendy,
            soft,
            spring,
            common,
            ..
        } if quadruped || creature.is_some() => {
            let c = creature.unwrap_or(demo::Creature::Dog);
            let mut guides = c.guides();
            components::set_limb_options(&mut guides, bendy, soft);
            if let Some(spring) = spring
                && components::set_hock_spring(&mut guides, spring) == 0
            {
                anyhow::bail!("the {} has no quad legs for --spring", c.name());
            }
            run(
                c.mesh(),
                &c.rig_name(),
                guides,
                &out,
                &common,
                Orientation::STANDARD,
            )
        }
        Command::Demo {
            out,
            arm_drop,
            bendy,
            soft,
            spring,
            common,
            ..
        } => {
            anyhow::ensure!(
                spring.is_none(),
                "the humanoid demo has no quad legs for --spring (try --creature horse)"
            );
            let mesh = demo::humanoid(arm_drop);
            let mut guides = detect(&mesh);
            components::set_limb_options(&mut guides, bendy, soft);
            run(
                mesh,
                "hive_demo",
                guides,
                &out,
                &common,
                Orientation::STANDARD,
            )
        }
        Command::Rebuild {
            rig,
            guides,
            out,
            keep_weights,
            common,
        } => {
            let old = load_rig(&rig)?;
            let dir = rig.parent().unwrap_or(Path::new("."));
            let guides_path =
                guides.unwrap_or_else(|| dir.join(format!("{}.guides.json", old.name)));
            let mut guides = load_guides(&guides_path)?;
            let out = out.unwrap_or_else(|| dir.to_path_buf());
            common.skin.apply(&mut guides.skin)?;
            if !keep_weights {
                print_warnings(&guides, &old.mesh);
            }
            let opts = common.options();
            let mut new = build_rig(old.mesh.clone(), &old.name, guides, opts)?;
            new.orientation = old.orientation;
            if keep_weights {
                // The old rig's automatic weights tell its hand edits apart.
                let base = old.auto_weights(opts.skin);
                let kept = rig::keep_weights(&old, Some(&base), &mut new);
                println!("Kept the hand-edited weights of {kept} vertices.");
            }
            for lost in hive_rig::shapes::carry_drivers(&old, &mut new) {
                eprintln!("warning: shape '{lost}' lost its driver: its bone is gone");
            }
            write(&new, &out, &common.output)
        }
        Command::Weights { action } => weights_command(action),
        Command::Shape { rig, action } => shape_command(&rig, action),
        Command::Add {
            guides,
            preset,
            side,
            parent,
            joints,
            id,
            controls,
        } => {
            let mut g = load_guides(&guides)?;
            let opts = AddOptions {
                side: side.map(|s| match s {
                    SideArg::Left => Side::Left,
                    SideArg::Right => Side::Right,
                }),
                parent,
                joints,
                id,
                controls,
            };
            let added = components::add_preset(&mut g, preset, &opts)?;
            g.save(&guides)?;
            println!("Added {}", added.join(", "));
            Ok(())
        }
        Command::Spine {
            guides,
            joints,
            spline,
            controls,
        } => {
            let mut g = Guides::load(&guides)?;
            let id = g
                .components
                .iter()
                .find(|c| c.kind == Kind::Spine)
                .map(|c| c.id.clone())
                .ok_or_else(|| anyhow::anyhow!("the layout has no spine"))?;
            if let Some(n) = joints {
                components::set_spine_joints(&mut g, &id, n)?;
            }
            if spline.is_some() || controls.is_some() {
                let on = spline.unwrap_or_else(|| g.component(&id).unwrap().options.spline);
                components::set_spine_spline(&mut g, &id, on, controls)?;
            }
            g.save(&guides)?;
            let joints = g.spine_joints(g.component(&id).unwrap());
            let o = &g.component(&id).unwrap().options;
            if o.spline {
                println!(
                    "{id}: {joints} joints on a curve with {} controls",
                    o.curve_controls(true)
                );
            } else {
                println!("{id}: {joints} joints with FK controls");
            }
            Ok(())
        }
        Command::Attach { guides, id, bone } => {
            let mut g = load_guides(&guides)?;
            components::attach(&mut g, &id, &bone)?;
            g.save(&guides)?;
            println!("{id} now hangs from {bone}");
            Ok(())
        }
        Command::Import {
            guides,
            from,
            parent,
        } => {
            let mut g = load_guides(&guides)?;
            let src = load_guides(&from)?;
            let added = components::import(&mut g, &src, &parent)?;
            g.save(&guides)?;
            println!("Imported {} from {from:?} onto {parent}", added.join(", "));
            Ok(())
        }
        Command::Remove { guides, id } => {
            let mut g = load_guides(&guides)?;
            let removed = components::remove(&mut g, &id)?;
            g.save(&guides)?;
            println!("Removed {}", removed.join(", "));
            Ok(())
        }
        Command::Components { guides } => {
            let g = load_guides(&guides)?;
            for c in &g.components {
                let kind = format!("{:?}", c.kind).to_lowercase();
                let parent = c.parent.as_deref().unwrap_or("-");
                let extra = match c.kind {
                    Kind::Chain => format!("{} bones", c.guides.len() - 1),
                    Kind::Spline => format!(
                        "{} bones, {} curve controls",
                        c.options.joints.unwrap_or(c.guides.len() - 1),
                        c.options.curve_controls(false)
                    ),
                    Kind::Spine if c.options.spline => format!(
                        "{} joints on a curve ({} controls)",
                        g.spine_joints(c),
                        c.options.curve_controls(true)
                    ),
                    Kind::Spine => format!(
                        "{} joints",
                        c.options.joints.unwrap_or(components::SPINE_JOINTS)
                    ),
                    _ => String::new(),
                };
                println!("{:<12} {:<7} on {:<12} {extra}", c.id, kind, parent);
            }
            Ok(())
        }
        Command::Check {
            file,
            mesh,
            fix,
            orient,
        } => check_command(&file, mesh.as_deref(), fix, orient),
        Command::Template { action } => template_command(action),
        Command::Naming {
            guides,
            preset,
            joint,
            control,
            left,
            right,
            center,
        } => {
            let mut g = load_guides(&guides)?;
            let mut n = match &preset {
                Some(p) => p.parse::<naming::Preset>()?.naming(),
                None => g.naming.clone(),
            };
            let mut changed = preset.is_some();
            for (field, value) in [
                (&mut n.joint, joint),
                (&mut n.control, control),
                (&mut n.left, left),
                (&mut n.right, right),
                (&mut n.center, center),
            ] {
                if let Some(v) = value {
                    *field = v;
                    changed = true;
                }
            }
            if changed {
                n.validate()?;
                g.naming = n.clone();
                g.save(&guides)?;
            }
            let label = n.preset().map_or("custom", |p| p.key());
            println!("naming: {label}");
            println!(
                "  joints:   {}  (upperarm_l -> {})",
                n.joint,
                n.bone("upperarm_l")
            );
            println!(
                "  controls: {}  (upperarm_l_fk_ctrl -> {})",
                n.control,
                n.control("upperarm_l_fk_ctrl")
            );
            Ok(())
        }
    }
}
