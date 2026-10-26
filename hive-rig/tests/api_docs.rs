//! docs/api.md's CLI reference is `hive-rig help-all --markdown`: this fails
//! when a command or flag changes and the page wasn't regenerated. Run with
//! `HIVE_UPDATE_API_DOCS=1` to rewrite the section.

use std::process::Command;

const START: &str = "<!-- cli-reference:start -->\n";
const END: &str = "<!-- cli-reference:end -->";

fn help_all(markdown: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hive-rig"));
    cmd.arg("help-all");
    if markdown {
        cmd.arg("--markdown");
    }
    let out = cmd.output().expect("running hive-rig help-all");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn api_docs_list_every_command_and_flag() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/api.md");
    let page = std::fs::read_to_string(&path).unwrap();
    let a = page
        .find(START)
        .expect("docs/api.md has the cli-reference:start marker")
        + START.len();
    let b = a + page[a..]
        .find(END)
        .expect("docs/api.md has the cli-reference:end marker");
    let want = help_all(true);
    if page[a..b] != want {
        if std::env::var_os("HIVE_UPDATE_API_DOCS").is_some() {
            std::fs::write(&path, format!("{}{want}{}", &page[..a], &page[b..])).unwrap();
            return;
        }
        let first = page[a..b]
            .lines()
            .zip(want.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(0);
        panic!(
            "docs/api.md's CLI reference is out of date (first difference at line {} of it: {:?}). \
             Run `HIVE_UPDATE_API_DOCS=1 cargo test --test api_docs` and commit the page.",
            first + 1,
            want.lines().nth(first).unwrap_or("")
        );
    }
    // Every command, nested ones too, has its heading.
    for name in [
        "rig",
        "demo",
        "rebuild",
        "weights copy",
        "shape sculpt",
        "scene build",
        "help-all",
    ] {
        assert!(want.contains(&format!("### `hive-rig {name}`")), "{name}");
    }
    // The other sections the page promises.
    for s in [
        "guides.json",
        ".hive.json",
        "scene.json",
        "edits.json",
        "\"pose\"",
        "HIVE_EXE",
        "examples/script_rig.rs",
    ] {
        assert!(page.contains(s), "docs/api.md doesn't mention {s}");
    }
}

#[test]
fn plain_help_all_has_every_command() {
    let text = help_all(false);
    for name in [
        "Usage: hive-rig rig",
        "Usage: hive-rig weights copy",
        "Usage: hive-rig scene new",
    ] {
        assert!(text.contains(name), "{name}");
    }
}
