import * as assert from "assert";
import * as path from "path";
import { findProjectRoot, runModeArgs } from "../../src/largoCore";

const SEP = path.sep;

function p(...parts: string[]): string {
  return SEP + parts.join(SEP);
}

describe("project root discovery", () => {
  it("finds Largo.toml in the starting directory", () => {
    const root = findProjectRoot(p("home", "me", "proj"), (candidate) =>
      candidate === path.join(p("home", "me", "proj"), "Largo.toml"),
    );
    assert.strictEqual(root, p("home", "me", "proj"));
  });

  it("walks up through nested directories", () => {
    const root = findProjectRoot(p("home", "me", "proj", "src", "deep", "deeper"), (candidate) =>
      candidate === path.join(p("home", "me", "proj"), "Largo.toml"),
    );
    assert.strictEqual(root, p("home", "me", "proj"));
  });

  it("returns undefined when no manifest exists anywhere", () => {
    const root = findProjectRoot(p("home", "me", "elsewhere"), () => false);
    assert.strictEqual(root, undefined);
  });

  it("terminates at the filesystem root instead of looping", () => {
    let calls = 0;
    const root = findProjectRoot(p("a", "b", "c"), () => {
      calls += 1;
      assert.ok(calls < 100, "the walk must terminate");
      return false;
    });
    assert.strictEqual(root, undefined);
  });

  it("prefers the nearest manifest when several exist up the tree", () => {
    const manifests = new Set([
      path.join(p("home", "me", "outer"), "Largo.toml"),
      path.join(p("home", "me", "outer", "inner"), "Largo.toml"),
    ]);
    const root = findProjectRoot(p("home", "me", "outer", "inner", "src"), (candidate) =>
      manifests.has(candidate),
    );
    assert.strictEqual(root, p("home", "me", "outer", "inner"));
  });
});

describe("run mode arguments", () => {
  it("interpret is the default and maps to --interpret", () => {
    assert.deepStrictEqual(runModeArgs("interpret"), ["run", "--interpret"]);
  });

  it("debug maps to a plain run", () => {
    assert.deepStrictEqual(runModeArgs("debug"), ["run"]);
  });

  it("release maps to run --release", () => {
    assert.deepStrictEqual(runModeArgs("release"), ["run", "--release"]);
  });

  it("an unknown mode falls back to interpret rather than guessing", () => {
    assert.deepStrictEqual(runModeArgs("turbo"), ["run", "--interpret"]);
  });
});
