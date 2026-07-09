import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";
import * as oniguruma from "vscode-oniguruma";
import * as vsctm from "vscode-textmate";

/**
 * Quickguide coverage ratchet — the grammar-side mirror of the repo's
 * guide_examples.rs pattern.
 *
 * Every backticked surface form in LOGOS_QUICKGUIDE.md's canonical/also-works
 * columns must produce at least one scope beyond `source.logicaffeine`, OR be
 * explicitly listed in PROSE_ONLY with a reason. The test fails in BOTH
 * drift directions: an unhighlighted form that isn't allowlisted ("cover it
 * or record why not"), and an allowlisted form that HAS gained highlighting
 * ("promote it out of the allowlist"). `(proposed)` cells are the spec, not
 * the language — skipped.
 */

/** Surface forms that legitimately tokenize as bare prose, with the reason. */
const PROSE_ONLY: Record<string, string> = {};

/** Table columns that hold LOGOS surface syntax (by header name). */
const SURFACE_COLUMNS = new Set([
  "canonical",
  "also works",
  "form",
  "symbolic",
  "english",
  "examples",
]);

const EXTENSION_ROOT = path.resolve(__dirname, "..", "..", "..");
const REPO_ROOT = path.resolve(EXTENSION_ROOT, "..", "..", "..");
const GRAMMAR_PATH = path.join(EXTENSION_ROOT, "syntaxes", "logicaffeine.tmLanguage.json");
const QUICKGUIDE_PATH = path.join(REPO_ROOT, "LOGOS_QUICKGUIDE.md");

interface Form {
  text: string;
  line: number;
}

function extractSurfaceForms(markdown: string): Form[] {
  const forms: Form[] = [];
  const lines = markdown.split("\n");
  let surfaceColumns: number[] = [];

  lines.forEach((line, index) => {
    const trimmed = line.trim();
    if (!trimmed.startsWith("|")) {
      surfaceColumns = [];
      return;
    }

    const cells = trimmed
      .split("|")
      .slice(1, -1)
      .map((c) => c.trim());

    // A header row selects which columns carry surface syntax.
    if (lines[index + 1]?.trim().match(/^\|[\s\-|]+\|$/)) {
      surfaceColumns = cells
        .map((cell, i) => (SURFACE_COLUMNS.has(cell.replace(/\*/g, "").toLowerCase()) ? i : -1))
        .filter((i) => i >= 0);
      return;
    }
    if (trimmed.match(/^\|[\s\-|]+\|$/) || surfaceColumns.length === 0) {
      return;
    }

    for (const column of surfaceColumns) {
      const cell = cells[column];
      if (!cell) {
        continue;
      }
      for (const alternative of cell.split("·")) {
        // Per-alternative, not per-cell: `xs[i]` shares a cell with a
        // (proposed) sibling and must still be checked.
        if (alternative.includes("(proposed)")) {
          continue;
        }
        for (const match of alternative.matchAll(/`([^`]+)`/g)) {
          const text = match[1].trim();
          // Ellipsis fragments describe shapes, not runnable surface.
          if (text.length > 0 && !text.includes("…")) {
            forms.push({ text, line: index + 1 });
          }
        }
      }
    }
  });

  return forms;
}

async function loadGrammar(): Promise<vsctm.IGrammar> {
  const wasmPath = require.resolve("vscode-oniguruma/release/onig.wasm");
  const wasmBin = fs.readFileSync(wasmPath).buffer;
  const onigLib = oniguruma.loadWASM(wasmBin).then(() => ({
    createOnigScanner: (patterns: string[]) => new oniguruma.OnigScanner(patterns),
    createOnigString: (s: string) => new oniguruma.OnigString(s),
  }));

  const registry = new vsctm.Registry({
    onigLib,
    loadGrammar: async (scopeName: string) => {
      if (scopeName === "source.logicaffeine") {
        return vsctm.parseRawGrammar(fs.readFileSync(GRAMMAR_PATH, "utf8"), GRAMMAR_PATH);
      }
      return null;
    },
  });

  const grammar = await registry.loadGrammar("source.logicaffeine");
  assert.ok(grammar, "the LOGOS grammar must load");
  return grammar!;
}

/** True when tokenizing produced any scope beyond the root. */
function highlights(grammar: vsctm.IGrammar, form: string): boolean {
  let ruleStack = vsctm.INITIAL;
  for (const line of form.split("\n")) {
    const result = grammar.tokenizeLine(line, ruleStack);
    for (const token of result.tokens) {
      if (token.scopes.some((scope) => scope !== "source.logicaffeine")) {
        return true;
      }
    }
    ruleStack = result.ruleStack;
  }
  return false;
}

describe("quickguide grammar coverage ratchet", function () {
  this.timeout(30_000);

  it("every canonical surface form highlights (or is allowlisted with a reason)", async () => {
    const markdown = fs.readFileSync(QUICKGUIDE_PATH, "utf8");
    const forms = extractSurfaceForms(markdown);
    assert.ok(
      forms.length > 80,
      `expected the quickguide to yield a rich form set, got ${forms.length} — did the table format change?`,
    );

    const grammar = await loadGrammar();

    const unhighlighted: string[] = [];
    const promotable: string[] = [];
    for (const form of forms) {
      const covered = highlights(grammar, form.text);
      const allowlisted = form.text in PROSE_ONLY;
      if (!covered && !allowlisted) {
        unhighlighted.push(`  quickguide:${form.line}  ${JSON.stringify(form.text)}`);
      }
      if (covered && allowlisted) {
        promotable.push(`  ${JSON.stringify(form.text)} — allowlisted but now highlights`);
      }
    }

    assert.deepStrictEqual(
      unhighlighted,
      [],
      `surface forms with no highlighting — cover them in the grammar or allowlist with a reason:\n${unhighlighted.join("\n")}`,
    );
    assert.deepStrictEqual(
      promotable,
      [],
      `allowlisted forms that now highlight — remove them from PROSE_ONLY:\n${promotable.join("\n")}`,
    );
  });
});
