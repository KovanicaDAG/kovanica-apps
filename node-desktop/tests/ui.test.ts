import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { fieldText, pathsReady } from "../src/format.ts";

const root = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(join(root, "../index.html"), "utf8");
const css = readFileSync(join(root, "../src/styles.css"), "utf8");
const main = readFileSync(join(root, "../src/main.ts"), "utf8");

test("missing monitor values are not shown as zero or synced", () => {
  assert.equal(fieldText({ kind: "unknown" }), "unknown");
  assert.equal(fieldText({ kind: "unavailable" }), "unavailable");
  assert.equal(fieldText({ kind: "value", value: "0" }), "0");
  assert.equal(fieldText({ kind: "value", value: null }), "unknown");
  assert.equal(pathsReady("", "/cfg", "/data"), false);
  assert.equal(pathsReady("/bin", "/cfg", "/data"), true);
});

test("the window has process controls and no wallet, stake, or mining controls", () => {
  assert.match(html, /id="dev-warning"/);
  assert.match(html, /role="alert"/);
  assert.match(html, /id="start-button"/);
  assert.match(html, /id="stop-button"/);
  assert.match(html, /id="log-viewer"/);
  assert.match(html, /tabindex="0"/);
  assert.doesNotMatch(html, /id="(validator-toggle|stake-button|mnemonic|seed-field)"/);
  assert.doesNotMatch(html, /type="password"/);
  assert.doesNotMatch(main, /wallet_create|sign_transfer|stake-button|run_validator|mnemonic/);
  assert.match(css, /:focus-visible/);
  assert.match(main, /document\.hidden/);
  assert.match(main, /refreshInFlight/);
});
