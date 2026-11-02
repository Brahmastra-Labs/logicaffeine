//! openroad_build_lock — the OpenLane image the comparative bench's Sky130 flows run, `scripts/openroad/`.
//!
//! OpenROAD 41a51eaf, the one OpenLane at the overlay's base carries, builds a net's buffered tree from its global
//! route by walking from the driver to every adjacent route point but the one it came from; on a route that closes a
//! cycle the walk never ends, and repair_timing overflows the stack. The bench runs that OpenROAD rebuilt with
//! `looped-route.patch`, which walks a route without a cycle exactly as before and one with a cycle once over a
//! spanning tree.
//!
//! A. the build is the pinned one with one patch: every source is pinned by SHA-256 and copied into the build, the
//!    OpenROAD commit is the one OpenLane at the overlay's base builds and its OpenSTA the one the power measurements
//!    run, the patch touches only the buffered-net walk, and the image is the base's with its `openroad` replaced;
//! B. the image is named by everything it is built from — the Dockerfile, the patch, the source list and the base —
//!    so a changed build is a new image, and an unchanged one the same;
//! C. the patch applies to the pinned source, and the commit it applies to is the one OpenLane at the base pins;
//! D. a bench run and a control flow in that image, refused before the flow when the name OpenLane launches (the image
//!    with the machine's architecture appended) is not built, and no other run names an image of its own; a run names
//!    an image only when its launcher writes `openlane_image` beside it;
//! E. a run's IR analysis is replayed where the run flowed: in its own OpenLane tree, in the image the run names, or in
//!    OpenLane as released for a run in `tools/OpenLane` that names none; anything else is refused before any
//!    container starts;
//! F. every ledger entry that names an image names the bench's.

mod common;
#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn base() -> String {
    read("scripts/openlane-overlay/BASE").trim().to_string()
}

/// `scripts/openroad/sources.txt`: (SHA-256, file, URL) a line.
fn sources() -> Vec<(String, String, String)> {
    read("scripts/openroad/sources.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 3, "a source line is its SHA-256, its file and its URL: {l:?}");
            (f[0].to_string(), f[1].to_string(), f[2].to_string())
        })
        .collect()
}

/// The commit a source file is pinned at: `<name>-<40 hex>.tar.gz`.
fn pinned_commit(prefix: &str) -> String {
    let (_, file, _) = sources().into_iter().find(|(_, f, _)| f.starts_with(prefix)).unwrap_or_else(|| panic!("{prefix} is pinned"));
    let commit = file.trim_start_matches(prefix).trim_end_matches(".tar.gz").to_string();
    assert!(commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()), "{file}: pinned at a commit");
    commit
}

fn image_name(scripts: &Path) -> String {
    let out = std::process::Command::new("bash")
        .arg(scripts.join("openroad/build-openroad.sh"))
        .arg("--name")
        .output()
        .expect("bash runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_bench_openroad_is_the_pinned_build_with_one_patch() {
    let srcs = sources();
    let mut files = std::collections::BTreeSet::new();
    for (sum, file, url) in &srcs {
        assert!(sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{file}: a SHA-256");
        assert!(!file.contains('/') && files.insert(file.clone()), "{file}: one plain file name, once");
        assert!(url.starts_with("https://"), "{file}: fetched over https");
    }
    let docker = read("scripts/openroad/openroad.Dockerfile");
    let mut copied = std::collections::BTreeSet::new();
    let mut statement: Option<String> = None;
    for line in docker.lines() {
        if line.starts_with("COPY ") && !line.starts_with("COPY --from=") {
            statement = Some(String::new());
        }
        if let Some(s) = statement.as_mut() {
            s.push_str(line.trim_end_matches('\\'));
            s.push(' ');
            if !line.ends_with('\\') {
                let words: Vec<&str> = s.split_whitespace().collect();
                copied.extend(words[1..words.len() - 1].iter().map(|w| w.to_string()));
                statement = None;
            }
        }
    }
    let mut built_from = files.clone();
    built_from.insert("looped-route.patch".to_string());
    assert_eq!(copied, built_from, "the build copies in every pinned source and the patch, and nothing else");
    let openroad = pinned_commit("OpenROAD-");
    assert!(
        docker.contains(&format!("sed -i \"s/GITDIR-NOTFOUND/{openroad}/\" cmake/GetGitRevisionDescription.cmake")),
        "the build reports the commit it is built from"
    );
    let opensta = read("scripts/opensta/build-opensta.sh");
    assert!(
        opensta.contains(&format!("COMMIT={}", pinned_commit("OpenSTA-"))),
        "OpenROAD's OpenSTA is the one the power measurements run"
    );
    assert!(docker.contains(" && patch -p1 < /src/looped-route.patch \\\n"), "the patch is applied");
    let patch = read("scripts/openroad/looped-route.patch");
    let touched: Vec<&str> = patch.lines().filter(|l| l.starts_with("+++ ")).collect();
    assert_eq!(touched, ["+++ b/src/rsz/src/BufferedNet.cc"], "the patch touches only the buffered-net walk");
    let last = docker.split("\nFROM ").last().unwrap();
    assert!(last.starts_with("${OPENLANE_IMAGE}\n"), "the image is the base's");
    let replaced: Vec<&str> = last.lines().filter(|l| l.starts_with("COPY --from=build ")).collect();
    assert_eq!(
        replaced,
        [
            "COPY --from=build /openroad/install/bin/openroad /build/bin/openroad",
            "COPY --from=build /usr/local/lib64/libortools.so.9 /build/lib/libortools.so.9",
        ],
        "only OpenROAD and the OR-Tools it links are replaced"
    );
    let script = read("scripts/openroad/build-openroad.sh");
    assert!(
        script.contains("--build-arg \"OPENLANE_IMAGE=ghcr.io/the-openroad-project/openlane:$BASE\""),
        "built over the image at the overlay's base"
    );
    let makefile_launch = "$(OPENLANE_IMAGE_NAME)-$(DOCKER_ARCH)";
    assert!(
        script.contains("ARCH=$(docker version --format '{{.Server.Arch}}')")
            && script.split("docker build -t").nth(1).is_some_and(|after| after.contains("docker tag \"$IMAGE\" \"$IMAGE-$ARCH\""))
            && script.matches("docker tag \"$IMAGE\" \"$IMAGE-$ARCH\"").count() == 2
            && script.contains("docker image inspect \"$IMAGE-$ARCH\""),
        "OpenLane's Makefile launches {makefile_launch}: the image is tagged with the machine's architecture too, and is \
         built only when that name exists"
    );
}

#[test]
fn the_image_is_named_by_everything_it_is_built_from() {
    let sb = tempfile::tempdir().unwrap();
    let scripts = sb.path().join("scripts");
    std::fs::create_dir_all(scripts.join("openroad")).unwrap();
    std::fs::create_dir_all(scripts.join("openlane-overlay")).unwrap();
    for f in ["build-openroad.sh", "openroad.Dockerfile", "looped-route.patch", "sources.txt"] {
        std::fs::copy(root().join("scripts/openroad").join(f), scripts.join("openroad").join(f)).unwrap();
    }
    std::fs::copy(root().join("scripts/openlane-overlay/BASE"), scripts.join("openlane-overlay/BASE")).unwrap();
    let name = image_name(&scripts);
    assert_eq!(name, image_name(&root().join("scripts")), "the name is the build's, wherever it is read");
    let prefix = format!("logos-openlane:{}-openroad-", &base()[..8]);
    let build = name.strip_prefix(&prefix).unwrap_or_else(|| panic!("{name}: named by the base"));
    assert!(build.len() == 12 && build.chars().all(|c| c.is_ascii_hexdigit()), "{name}: named by its build");
    for input in ["openroad/openroad.Dockerfile", "openroad/looped-route.patch", "openroad/sources.txt", "openlane-overlay/BASE"] {
        let path = scripts.join(input);
        let kept = std::fs::read(&path).unwrap();
        let changed = if input.ends_with("BASE") {
            b"0123456789abcdef0123456789abcdef01234567\n".to_vec()
        } else {
            [kept.as_slice(), b"\n"].concat()
        };
        std::fs::write(&path, &changed).unwrap();
        assert_ne!(image_name(&scripts), name, "{input}: a changed input is a new image");
        std::fs::write(&path, &kept).unwrap();
        assert_eq!(image_name(&scripts), name, "{input}: the same inputs, the same image");
    }
}

#[test]
fn the_patch_applies_to_the_pinned_source_of_the_openroad_the_base_builds() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let metadata = read("tools/OpenLane/dependencies/tool_metadata.yml");
    let openroad = pinned_commit("OpenROAD-");
    let entry = metadata.split("- name: ").find(|e| e.starts_with("openroad_app")).expect("OpenLane pins openroad_app");
    assert!(entry.contains(&format!("commit: {openroad}")), "the pinned OpenROAD is the one OpenLane at the base builds");
    let (sum, file, _) = sources().into_iter().find(|(_, f, _)| f.starts_with("OpenROAD-")).unwrap();
    let tarball = root().join("tools/openroad").join(&file);
    assert!(tarball.is_file(), "{}: fetch it with scripts/openroad/build-openroad.sh", tarball.display());
    let hashed = std::process::Command::new("sha256sum").arg(&tarball).output().unwrap();
    assert!(String::from_utf8_lossy(&hashed.stdout).starts_with(&sum), "{file}: the pinned source");
    let work = tempfile::tempdir().unwrap();
    let member = format!("OpenROAD-{openroad}/src/rsz/src/BufferedNet.cc");
    let out = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(&tarball)
        .args(["-C"])
        .arg(work.path())
        .args(["--strip-components=1", &member])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let applied = std::process::Command::new("patch")
        .args(["-p1", "--forward", "-i"])
        .arg(root().join("scripts/openroad/looped-route.patch"))
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(applied.status.success(), "{}{}", String::from_utf8_lossy(&applied.stdout), String::from_utf8_lossy(&applied.stderr));
    let walked = std::fs::read_to_string(work.path().join("src/rsz/src/BufferedNet.cc")).unwrap();
    assert!(
        walked.contains("bool looped = routeHasCycle(adjacents);") && walked.contains("looped ? &visited : nullptr"),
        "the walk takes a visited set exactly when the route closes a cycle"
    );
}

#[test]
fn a_bench_run_and_a_control_flow_in_the_bench_image_and_no_other_run_names_one() {
    let runner = read("scripts/openlane-run.sh");
    let lines: Vec<&str> = runner.lines().collect();
    let launch = lines.iter().position(|l| l.starts_with("( cd \"$OL\" && sg docker -c \"make quick_run")).expect("the launch");
    assert!(
        lines[launch].contains("${OL_IMAGE:+ OPENLANE_IMAGE_NAME=$OL_IMAGE}"),
        "a run names an image only when it has one"
    );
    let guard = [
        "if [ \"$OL_STOCK\" = 1 ] || [ \"$OL_BENCH\" = 1 ]; then",
        "  OL_IMAGE=$(bash \"$ROOT/scripts/openroad/build-openroad.sh\" --name)",
        "  if ! sg docker -c \"bash $ROOT/scripts/openroad/build-openroad.sh --built\" >/dev/null 2>&1; then",
        "    echo \"FATAL: the bench's OpenLane image $OL_IMAGE is not built (scripts/openroad/build-openroad.sh)\" >&2",
        "    exit 1",
        "  fi",
        "fi",
    ];
    assert_eq!(&lines[launch - guard.len()..launch], guard, "a bench run and a control take the bench image, refused when it is not built");
    let assigned: Vec<&str> = lines.iter().copied().filter(|l| l.trim_start().starts_with("OL_IMAGE=")).collect();
    assert_eq!(
        assigned,
        ["OL_IMAGE=\"\"", "  OL_IMAGE=$(bash \"$ROOT/scripts/openroad/build-openroad.sh\" --name)"],
        "no other run names an image of its own"
    );
    let named: Vec<&str> = lines.iter().copied().filter(|l| l.contains("openlane_image")).filter(|l| !l.trim_start().starts_with('#')).collect();
    assert_eq!(
        named,
        ["if [ -n \"$OL_IMAGE\" ]; then printf '%s\\n' \"$OL_IMAGE\" > \"${RUN%/}/openlane_image\"; fi"],
        "a run names an image only when its launcher ran one of its own"
    );
}

/// The anchored IR replay (`scripts/anchor-ir-drop.sh`) runs where its run flowed: a run is
/// `<tree>/designs/<design>/runs/<run>` in an OpenLane tree (one with `scripts/openroad/irdrop.tcl` to replay), the
/// replay mounts that tree as the flow did, and launches the image the run names in `openlane_image` beside it (with
/// the machine's architecture appended, refused when it is not on the machine or is no name `build-openroad.sh`
/// prints), or OpenLane as released for a run in `tools/OpenLane` that names none. A run anywhere else that names none
/// is refused — the bench's trees among them, whose launcher always names its image — as is a path that is no run, or
/// a tree with nothing to replay, each before any container starts. A tree reached through a symbolic link (a copy of
/// the repository whose `tools` links the checkout's) is the same tree.
#[test]
fn an_ir_replay_runs_in_the_tree_and_the_image_its_run_flowed_in() {
    use std::os::unix::fs::PermissionsExt;
    let sb = std::env::temp_dir().join(format!("ir-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in ["scripts/openroad", "scripts/openlane-overlay", "bin", "real-tools"] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    for f in [
        "scripts/anchor-ir-drop.sh",
        "scripts/gen-vsrc-loc.py",
        "scripts/openroad/build-openroad.sh",
        "scripts/openroad/openroad.Dockerfile",
        "scripts/openroad/looped-route.patch",
        "scripts/openroad/sources.txt",
        "scripts/openlane-overlay/BASE",
    ] {
        std::fs::copy(root().join(f), sb.join(f)).unwrap_or_else(|e| panic!("copy {f}: {e}"));
    }
    std::os::unix::fs::symlink(sb.join("real-tools"), sb.join("tools")).unwrap();
    for tree in ["tools/OpenLane", "tools/qor/openlane-bench", "tools/qor/openlane-stock", "tools/clockpush/openlane"] {
        std::fs::create_dir_all(sb.join(tree).join("scripts/openroad")).unwrap();
        std::fs::write(sb.join(tree).join("scripts/openroad/irdrop.tcl"), "# the flow's irdrop step\n").unwrap();
    }
    let sg = sb.join("bin/sg");
    std::fs::write(
        &sg,
        r#"#!/usr/bin/env python3
import os, re, shlex, sys
cmd = sys.argv[sys.argv.index("-c") + 1]
open(os.environ["SG_LOG"], "a").write(" ".join(cmd.split()) + "\n")
if cmd.startswith("docker version"):
    print("amd64")
    sys.exit(0)
if cmd.startswith("docker image inspect "):
    sys.exit(0 if cmd.split()[3] in os.environ.get("FAKE_IMAGES", "").split() else 1)
if cmd.lstrip().startswith("docker run"):
    words = shlex.split(cmd)
    mounts = {c: h for h, c in (words[i + 1].split(":", 1) for i, w in enumerate(words) if w == "-v")}
    host = lambda path: mounts["/openlane"] + path[len("/openlane"):]
    prefix = re.search(r"_tmp_save_rpt_prefix\) (\S+)", open(host(words[-1])).read()).group(1)
    for net, volts in (("VPWR", "1.7712"), ("VGND", "0.0100")):
        open(host(prefix) + "-%s.rpt" % net, "w").write("Instance,Terminal,X,Y,Voltage\nu1,%s,1,1,%s\n" % (net, volts))
    print("OpenROAD 41a51eaf")
    sys.exit(0)
sys.exit(1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&sg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let make_run = |tree: &str, named: Option<&str>| -> PathBuf {
        let run = sb.join(tree).join("designs/logos_toy/runs/RUN_1");
        let _ = std::fs::remove_dir_all(&run);
        std::fs::create_dir_all(run.join("reports/signoff")).unwrap();
        std::fs::write(
            run.join("config.tcl"),
            "set ::env(DESIGN_NAME) \"toy\"\nset ::env(DIE_AREA) \"0 0 100 120\"\nset ::env(FP_PDN_CORE_RING) \"1\"\n\
             set ::env(FP_PDN_VPITCH) \"50\"\nset ::env(PDK) \"sky130A\"\nset ::env(STD_CELL_LIBRARY) \"sky130_fd_sc_hd\"\n",
        )
        .unwrap();
        if let Some(image) = named {
            std::fs::write(run.join("openlane_image"), format!("{image}\n")).unwrap();
        }
        for net in ["VPWR", "VGND"] {
            let rpt = run.join(format!("reports/signoff/34-irdrop-{net}.rpt"));
            std::fs::write(&rpt, format!("Instance,Terminal,X,Y,Voltage\nu1,{net},1,1,1.7000\n")).unwrap();
            let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
            std::fs::File::options().write(true).open(&rpt).unwrap().set_modified(an_hour_ago).unwrap();
        }
        run
    };
    let bench = image_name(&root().join("scripts"));
    let on_machine = format!("{bench}-amd64");
    let replay = |run: &Path, images: &str| -> (i32, String, Vec<String>) {
        let log = sb.join("sg.log");
        let _ = std::fs::remove_file(&log);
        let o = std::process::Command::new("bash")
            .arg(sb.join("scripts/anchor-ir-drop.sh"))
            .arg(run)
            .env_clear()
            .env("PATH", format!("{}:{}", sb.join("bin").display(), std::env::var("PATH").unwrap()))
            .env("HOME", sb.as_os_str())
            .env("SG_LOG", &log)
            .env("FAKE_IMAGES", images)
            .current_dir(&sb)
            .output()
            .expect("run the replay");
        let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_string).collect();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
            calls,
        )
    };
    let released = format!("ghcr.io/the-openroad-project/openlane:{}-amd64", base());
    for (tree, given, named, image) in [
        ("tools/OpenLane", "tools/OpenLane", None, released.as_str()),
        ("tools/OpenLane", "real-tools/OpenLane", None, released.as_str()),
        ("tools/OpenLane", "tools/OpenLane", Some(bench.as_str()), on_machine.as_str()),
        ("tools/qor/openlane-bench", "tools/qor/openlane-bench", Some(bench.as_str()), on_machine.as_str()),
        ("tools/qor/openlane-stock", "real-tools/qor/openlane-stock", Some(bench.as_str()), on_machine.as_str()),
        ("tools/clockpush/openlane", "tools/clockpush/openlane", Some(bench.as_str()), on_machine.as_str()),
    ] {
        make_run(tree, named);
        let run = sb.join(given).join("designs/logos_toy/runs/RUN_1");
        let (code, out, calls) = replay(&run, &on_machine);
        let launch: Vec<&String> = calls.iter().filter(|c| c.starts_with("docker run")).collect();
        assert_eq!(launch.len(), 1, "{given} {named:?}: one replay container:\n{out}\n{calls:?}");
        let mount = format!("-v {}:/openlane ", sb.join(given).display());
        assert!(launch[0].contains(&mount), "{given}: the replay mounts the run's own tree ({mount}): {}", launch[0]);
        assert!(
            launch[0].contains(&format!(" {image} openroad -exit ")),
            "{given} {named:?}: the replay launches {image}: {}",
            launch[0]
        );
        assert!(
            code == 0 && out.contains("ANCHORED: 1 instances, worst IR drop 28.80 mV"),
            "{given} {named:?}: exit {code}, the anchored report read back:\n{out}"
        );
        assert!(
            std::fs::read_to_string(run.join("reports/signoff/34-irdrop-VPWR.rpt.unanchored")).is_ok_and(|t| t.contains("1.7000")),
            "{given}: the unanchored report is kept beside the anchored one"
        );
    }

    let refused = |run: PathBuf, images: &str, why: &str| {
        let (code, out, calls) = replay(&run, images);
        assert!(code != 0 && out.contains(why), "{}: refused naming {why:?}, exit {code}:\n{out}", run.display());
        assert!(!calls.iter().any(|c| c.starts_with("docker run")), "{}: no container starts: {calls:?}", run.display());
        assert!(!calls.iter().any(|c| c.contains("pwned")), "{}: a marker never reaches a shell: {calls:?}", run.display());
    };
    for tree in ["tools/qor/openlane-bench", "tools/qor/openlane-stock", "tools/clockpush/openlane"] {
        refused(make_run(tree, None), &on_machine, "names no image");
    }
    refused(make_run("tools/qor/openlane-bench", Some(bench.as_str())), "", "is not on this machine");
    refused(make_run("tools/OpenLane", Some("logos-openlane:latest; touch pwned")), &on_machine, "is no image build-openroad.sh names");
    assert!(!sb.join("pwned").exists(), "a marker is a name, never a command");
    refused(make_run("tools/OpenLane", Some("ghcr.io/the-openroad-project/openlane:latest")), &on_machine, "is no image build-openroad.sh names");
    let other_build = format!("{}000000000000", bench.trim_end_matches(|c: char| c.is_ascii_hexdigit()));
    refused(make_run("tools/qor/openlane-bench", Some(other_build.as_str())), &format!("{other_build}-amd64"), "is no image build-openroad.sh names");
    refused(make_run("elsewhere/ol", Some(bench.as_str())), &on_machine, "is not an OpenLane tree");
    refused(sb.join("tools/qor/openlane-bench/designs/logos_toy"), &on_machine, "is not a run of an OpenLane tree");
    std::fs::remove_dir_all(&sb).ok();
}

/// Every Sky130 ledger entry that names the image its run flowed in (`openlane_image`, recorded from the run's
/// marker) names the bench's: the image `scripts/openroad/build-openroad.sh` builds, and no other.
#[test]
fn every_ledger_entry_that_names_an_image_names_the_bench_image() {
    let bench = image_name(&root().join("scripts"));
    let mut named = 0;
    for ledger in sky130_ledgers::ledgers(&root(), None) {
        let Ok(text) = std::fs::read_to_string(&ledger) else { continue };
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", ledger.display()));
        for (block, entry) in doc["blocks"].as_object().into_iter().flatten() {
            if let Some(image) = entry.get("openlane_image") {
                named += 1;
                assert_eq!(
                    image.as_str(),
                    Some(bench.as_str()),
                    "{} {block}: flowed in {image}, not the bench image",
                    ledger.display()
                );
            }
        }
    }
    eprintln!("{named} entries name the bench image");
}
