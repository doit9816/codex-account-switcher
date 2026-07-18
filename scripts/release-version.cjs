const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const version = process.argv[2];
const dryRun = process.argv.includes("--dry-run");

function fail(message) {
  console.error(`release-version: ${message}`);
  process.exit(1);
}

if (!version || version === "--dry-run") {
  fail("usage: npm run release:version -- <x.y.z> [-- --dry-run]");
}

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`invalid semver: ${version}`);
}

function filePath(relativePath) {
  return path.join(root, relativePath);
}

function readText(relativePath) {
  return fs.readFileSync(filePath(relativePath), "utf8");
}

function writeText(relativePath, contents) {
  if (dryRun) return;
  fs.writeFileSync(filePath(relativePath), contents, "utf8");
}

function updateJsonVersion(relativePath) {
  const file = filePath(relativePath);
  const json = JSON.parse(fs.readFileSync(file, "utf8"));
  const previous = json.version;
  json.version = version;
  if (!dryRun) {
    fs.writeFileSync(file, `${JSON.stringify(json, null, 2)}\n`, "utf8");
  }
  return `${relativePath}: ${previous} -> ${version}`;
}

function replaceRequired(relativePath, pattern, replacement) {
  const contents = readText(relativePath);
  if (!pattern.test(contents)) {
    fail(`could not find version field in ${relativePath}`);
  }
  const previous = contents.match(pattern)?.[1];
  writeText(relativePath, contents.replace(pattern, replacement));
  return `${relativePath}: ${previous} -> ${version}`;
}

function updateCargoLock() {
  const relativePath = "src-tauri/Cargo.lock";
  const contents = readText(relativePath);
  const pattern = /(\[\[package\]\]\r?\nname = "codex-account-switcher"\r?\nversion = ")([^"]+)(")/;
  const match = contents.match(pattern);
  if (!match) {
    fail(`could not find codex-account-switcher package in ${relativePath}`);
  }
  const updated = contents.replace(pattern, `$1${version}$3`);
  writeText(relativePath, updated);
  return `${relativePath}: ${match[2]} -> ${version}`;
}

const changes = [
  updateJsonVersion("package.json"),
  updateJsonVersion("package-lock.json"),
  replaceRequired("src-tauri/Cargo.toml", /^version = "([^"]+)"/m, `version = "${version}"`),
  updateCargoLock(),
  updateJsonVersion("src-tauri/tauri.conf.json")
];

const tag = `codex-account-switcher-v${version}`;
console.log(`${dryRun ? "Checked" : "Updated"} release version ${version}`);
for (const change of changes) {
  console.log(`- ${change}`);
}
console.log(`Next tag: ${tag}`);
