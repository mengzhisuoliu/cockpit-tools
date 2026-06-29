#!/usr/bin/env node

const { execFileSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const INDEX_PATH = path.join(ROOT, 'platform-packages', 'index.json');
const INDEX_SEED_PATH = path.join(ROOT, 'platform-packages', 'index.seed.json');
const DEFAULT_DIST_DIR = path.join(ROOT, 'platform-packages', 'dist');
const STAGING_ROOT = path.join(ROOT, '.tmp', 'platform-package-staging');
const WORKSPACE_CARGO_TOML_PATH = path.join(ROOT, 'Cargo.toml');
const WINDOWS_COMMON_CONTROLS_BUILD_RULE_PATH = path.join(ROOT, 'crates', 'adapter-windows-common-controls-build.rs');
const WINDOWS_COMMON_CONTROLS_RC_PATH = path.join(ROOT, 'crates', 'windows-common-controls-v6.rc');
const WINDOWS_COMMON_CONTROLS_MANIFEST_PATH = path.join(ROOT, 'crates', 'windows-common-controls-v6.manifest');
const WINDOWS_ADAPTER_BUILD_RS_INCLUDE = 'include!("../adapter-windows-common-controls-build.rs");';

function fail(message) {
  console.error(message);
  process.exit(1);
}

function usage() {
  console.log(`Usage:
  node scripts/package-platform-package.cjs --platform <id> [options]

Options:
  --os <macos|windows|linux>          Target platform OS. Defaults to current OS.
  --arch <aarch64|x86_64>             Target platform arch. Defaults to current arch.
  --adapter-bin-dir <path>            Directory containing built adapter binary.
  --dist-dir <path>                   Output directory. Defaults to platform-packages/dist.
  --filename-template <legacy|os-arch> Zip name mode. Defaults to legacy.
  --metadata-out <path>               Write artifact metadata JSON.
  --download-url <url>                Override metadata downloadUrl.
  --update-index                      Update platform-packages/index.json for this artifact.
`);
}

function parseArgs(argv) {
  const args = {
    distDir: DEFAULT_DIST_DIR,
    filenameTemplate: 'legacy',
    updateIndex: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') {
      usage();
      process.exit(0);
    }
    if (arg === '--update-index') {
      args.updateIndex = true;
      continue;
    }
    const next = argv[index + 1];
    if (!next || next.startsWith('--')) {
      fail(`Missing value for ${arg}`);
    }
    index += 1;
    if (arg === '--platform') args.platformId = next;
    else if (arg === '--os') args.os = normalizeOs(next);
    else if (arg === '--arch') args.arch = normalizeArch(next);
    else if (arg === '--adapter-bin-dir') args.adapterBinDir = path.resolve(ROOT, next);
    else if (arg === '--dist-dir') args.distDir = path.resolve(ROOT, next);
    else if (arg === '--filename-template') args.filenameTemplate = next;
    else if (arg === '--metadata-out') args.metadataOut = path.resolve(ROOT, next);
    else if (arg === '--download-url') args.downloadUrl = next;
    else fail(`Unknown argument: ${arg}`);
  }

  if (!args.platformId) fail('Missing --platform <id>');
  args.os = normalizeOs(args.os || process.platform);
  args.arch = normalizeArch(args.arch || process.arch);
  if (!['legacy', 'os-arch'].includes(args.filenameTemplate)) {
    fail('--filename-template must be legacy or os-arch');
  }
  return args;
}

function normalizeOs(value) {
  if (value === 'darwin') return 'macos';
  if (value === 'win32') return 'windows';
  if (value === 'macos' || value === 'windows' || value === 'linux') return value;
  fail(`Unsupported OS: ${value}`);
}

function normalizeArch(value) {
  if (value === 'arm64') return 'aarch64';
  if (value === 'x64') return 'x86_64';
  if (value === 'aarch64' || value === 'x86_64') return value;
  fail(`Unsupported arch: ${value}`);
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch (error) {
    fail(`${label}: failed to read JSON: ${error.message}`);
  }
}

function readText(filePath, label) {
  try {
    return fs.readFileSync(filePath, 'utf8');
  } catch (error) {
    fail(`${label}: failed to read file: ${error.message}`);
  }
}

function safeRelativePath(relativePath, label) {
  if (!relativePath || typeof relativePath !== 'string') {
    fail(`${label}: path is required`);
  }
  if (path.isAbsolute(relativePath) || relativePath.includes('\0')) {
    fail(`${label}: unsafe path ${relativePath}`);
  }
  const normalized = path.normalize(relativePath);
  if (normalized === '..' || normalized.startsWith(`..${path.sep}`)) {
    fail(`${label}: unsafe path ${relativePath}`);
  }
  return normalized;
}

function sha256(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

function displayPath(filePath) {
  const relativePath = path.relative(ROOT, filePath);
  if (relativePath === '' || (!relativePath.startsWith('..') && !path.isAbsolute(relativePath))) {
    return relativePath;
  }
  return filePath;
}

function adapterEntryForOs(adapter, os) {
  if (!adapter) return null;
  if (os === 'macos') return adapter.macosEntry || adapter.entry;
  if (os === 'windows') return adapter.windowsEntry || adapter.entry;
  if (os === 'linux') return adapter.linuxEntry || adapter.entry;
  return adapter.entry;
}

function expectedAdapterCrateName(platformId) {
  if (platformId === 'claude_manager') return 'cockpit-claude-adapter';
  return `cockpit-${platformId.replace(/_/g, '-')}-adapter`;
}

function rustTargetFor(os, arch) {
  if (os === 'macos') return `${arch}-apple-darwin`;
  if (os === 'windows') return `${arch}-pc-windows-msvc`;
  if (os === 'linux') return `${arch}-unknown-linux-gnu`;
  fail(`Unsupported OS for Rust target: ${os}`);
}

function assertFile(filePath, label) {
  if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
    fail(`${label}: missing file ${path.relative(ROOT, filePath)}`);
  }
}

function assertDir(filePath, label) {
  if (!fs.existsSync(filePath) || !fs.statSync(filePath).isDirectory()) {
    fail(`${label}: missing directory ${path.relative(ROOT, filePath)}`);
  }
}

function assertIncludes(label, source, expected) {
  if (!source.includes(expected)) {
    fail(`${label}: missing ${expected}`);
  }
}

function createPackageStagingRoot(platformId) {
  const safeId = platformId.replace(/[^a-zA-Z0-9._-]/g, '_');
  const stagingRoot = path.join(STAGING_ROOT, `${safeId}-${process.pid}-${Date.now()}`);
  fs.rmSync(stagingRoot, { recursive: true, force: true });
  fs.mkdirSync(stagingRoot, { recursive: true });
  return stagingRoot;
}

function shouldSkipPackageSourceEntry(relativePath, dirent) {
  const parts = relativePath.split(path.sep).filter(Boolean);
  const name = dirent.name;
  const isTopLevel = parts.length === 1;

  if (parts[0] === 'adapter') return true;
  if (name === '.DS_Store') return true;

  if (dirent.isDirectory()) {
    if (['.git', 'node_modules'].includes(name) || name.endsWith('.dSYM')) return true;
    return isTopLevel && ['bootstrap', 'dist', 'dist-ci', 'test'].includes(name);
  }

  if (!dirent.isFile()) return true;
  return (
    name.endsWith('.map')
    || name.endsWith('.zip')
    || name.endsWith('.zip.part')
    || name.endsWith('.part')
    || name.endsWith('.pdb')
  );
}

function copyPackageSourceToStaging(sourceRoot, targetRoot, relativeRoot = '') {
  for (const dirent of fs.readdirSync(path.join(sourceRoot, relativeRoot), { withFileTypes: true })) {
    const relativePath = path.join(relativeRoot, dirent.name);
    if (shouldSkipPackageSourceEntry(relativePath, dirent)) continue;

    const sourcePath = path.join(sourceRoot, relativePath);
    const targetPath = path.join(targetRoot, relativePath);
    if (dirent.isDirectory()) {
      fs.mkdirSync(targetPath, { recursive: true });
      copyPackageSourceToStaging(sourceRoot, targetRoot, relativePath);
    } else if (dirent.isFile()) {
      fs.mkdirSync(path.dirname(targetPath), { recursive: true });
      fs.copyFileSync(sourcePath, targetPath);
      fs.chmodSync(targetPath, fs.statSync(sourcePath).mode & 0o777);
    }
  }
}

function verifyWindowsAdapterManifestBuild(platformId, crateTomlPath) {
  assertFile(WINDOWS_COMMON_CONTROLS_BUILD_RULE_PATH, 'shared Windows adapter build rule');
  assertFile(WINDOWS_COMMON_CONTROLS_RC_PATH, 'Windows Common Controls v6 resource file');
  assertFile(WINDOWS_COMMON_CONTROLS_MANIFEST_PATH, 'Windows Common Controls v6 manifest');

  const workspaceCargoToml = readText(WORKSPACE_CARGO_TOML_PATH, 'workspace Cargo.toml');
  if (!/^\s*embed-resource\s*=\s*["{]/m.test(workspaceCargoToml)) {
    fail('workspace Cargo.toml: missing embed-resource workspace dependency');
  }

  const buildRule = readText(WINDOWS_COMMON_CONTROLS_BUILD_RULE_PATH, 'shared Windows adapter build rule');
  assertIncludes('shared Windows adapter build rule', buildRule, 'windows-common-controls-v6.rc');
  assertIncludes('shared Windows adapter build rule', buildRule, 'embed_resource::compile');
  assertIncludes('shared Windows adapter build rule', buildRule, 'manifest_required');

  const resource = readText(WINDOWS_COMMON_CONTROLS_RC_PATH, 'Windows Common Controls v6 resource file');
  assertIncludes('Windows Common Controls v6 resource file', resource, 'RT_MANIFEST');
  assertIncludes('Windows Common Controls v6 resource file', resource, 'windows-common-controls-v6.manifest');

  const manifest = readText(WINDOWS_COMMON_CONTROLS_MANIFEST_PATH, 'Windows Common Controls v6 manifest');
  assertIncludes('Windows Common Controls v6 manifest', manifest, 'Microsoft.Windows.Common-Controls');
  assertIncludes('Windows Common Controls v6 manifest', manifest, 'version="6.0.0.0"');

  const crateDir = path.dirname(crateTomlPath);
  const crateCargoToml = readText(crateTomlPath, `${platformId}: adapter Cargo.toml`);
  const buildRsPath = path.join(crateDir, 'build.rs');
  assertFile(buildRsPath, `${platformId}: adapter build.rs`);
  if (!/^\s*build\s*=\s*["']build\.rs["']/m.test(crateCargoToml)) {
    fail(`${platformId}: adapter Cargo.toml must declare build = "build.rs"`);
  }
  if (!/^\s*embed-resource\s*=\s*\{\s*workspace\s*=\s*true\s*\}/m.test(crateCargoToml)) {
    fail(`${platformId}: adapter Cargo.toml must use embed-resource workspace build dependency`);
  }

  const buildRs = readText(buildRsPath, `${platformId}: adapter build.rs`);
  assertIncludes(`${platformId}: adapter build.rs`, buildRs, WINDOWS_ADAPTER_BUILD_RS_INCLUDE);
}

function copyAdapterIfAvailable(sourcePackageRoot, stagedPackageRoot, manifest, os, adapterBinDir) {
  if (!manifest.adapter) {
    if (manifest.installKind === 'sidecarAdapter') {
      fail(`${manifest.id}: sidecarAdapter package is missing adapter`);
    }
    return null;
  }

  const entry = safeRelativePath(adapterEntryForOs(manifest.adapter, os), `${manifest.id}: adapter entry`);
  const targetPath = path.join(stagedPackageRoot, entry);
  const targetBasename = path.basename(entry);
  const sourcePath = adapterBinDir
    ? path.join(adapterBinDir, targetBasename)
    : path.join(sourcePackageRoot, entry);

  if (!fs.existsSync(sourcePath)) {
    fail(`${manifest.id}: built adapter not found at ${path.relative(ROOT, sourcePath)}`);
  }

  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.copyFileSync(sourcePath, targetPath);
  assertFile(targetPath, `${manifest.id}: adapter`);
  if (os !== 'windows') {
    fs.chmodSync(targetPath, 0o755);
  }
  return entry;
}

function copyCodexRuntimeHelpers(packageRoot, manifest, os, arch) {
  if (manifest.id !== 'codex') return [];

  const adapterEntry = safeRelativePath(adapterEntryForOs(manifest.adapter, os), 'codex: adapter entry');
  const adapterDir = path.dirname(path.join(packageRoot, adapterEntry));
  const extension = os === 'windows' ? '.exe' : '';
  const sourcePath = path.join(
    ROOT,
    'sidecars',
    'cockpit-cliproxy',
    'bin',
    `cockpit-cliproxy-${rustTargetFor(os, arch)}${extension}`,
  );
  const targetPath = path.join(adapterDir, `cockpit-cliproxy${extension}`);

  assertFile(sourcePath, 'codex: cockpit-cliproxy helper');
  fs.mkdirSync(adapterDir, { recursive: true });
  fs.copyFileSync(sourcePath, targetPath);
  if (os !== 'windows') {
    fs.chmodSync(targetPath, 0o755);
  }
  return [path.relative(packageRoot, targetPath)];
}

function createZip(packageRoot, zipPath) {
  fs.rmSync(zipPath, { force: true });
  fs.mkdirSync(path.dirname(zipPath), { recursive: true });

  if (process.platform === 'win32') {
    execFileSync('powershell', [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      '$ErrorActionPreference = "Stop"; Compress-Archive -Path * -DestinationPath $env:ZIP_PATH -Force',
    ], {
      cwd: packageRoot,
      env: { ...process.env, ZIP_PATH: zipPath },
      stdio: 'inherit',
    });
    return;
  }

  execFileSync('zip', ['-qr', zipPath, '.', '-x', '**/.DS_Store'], {
    cwd: packageRoot,
    stdio: 'inherit',
  });
}

function zipNameFor(platformId, version, os, arch, template) {
  if (template === 'os-arch') return `${platformId}-${version}-${os}-${arch}.zip`;
  return `${platformId}-${version}.zip`;
}

function replaceZipName(downloadUrl, zipName) {
  if (!downloadUrl || typeof downloadUrl !== 'string') return `platform-packages/dist/${zipName}`;
  const clean = downloadUrl.split('?')[0].split('#')[0];
  const index = clean.lastIndexOf('/');
  if (index < 0) return zipName;
  return `${downloadUrl.slice(0, index + 1)}${zipName}`;
}

function updateIndex(index, platformId, os, arch, metadata, manifest) {
  const pkg = (index.packages || []).find((item) => item.id === platformId);
  if (!pkg) fail(`${platformId}: missing from platform-packages/index.json`);

  for (const key of [
    'platformId',
    'version',
    'apiVersion',
    'minCoreVersion',
    'displayName',
    'entry',
    'packageMode',
    'installKind',
    'adapter',
    'ui',
    'capabilities',
    'changelog',
    'contributions',
  ]) {
    if (Object.prototype.hasOwnProperty.call(manifest, key)) {
      pkg[key] = manifest[key];
    }
  }

  if (!Array.isArray(pkg.artifacts)) pkg.artifacts = [];
  const artifactIndex = pkg.artifacts.findIndex((artifact) => artifact.os === os && artifact.arch === arch);
  const artifact = {
    os,
    arch,
    downloadUrl: metadata.downloadUrl,
    downloadSizeBytes: metadata.downloadSizeBytes,
    sha256: metadata.sha256,
  };
  if (artifactIndex >= 0) pkg.artifacts[artifactIndex] = artifact;
  else pkg.artifacts.push(artifact);

  if (artifactIndex === 0 || (artifactIndex < 0 && pkg.artifacts.length === 1)) {
    pkg.downloadUrl = artifact.downloadUrl;
    pkg.downloadSizeBytes = artifact.downloadSizeBytes;
    pkg.sha256 = artifact.sha256;
  }
  const content = `${JSON.stringify(index, null, 2)}\n`;
  fs.writeFileSync(INDEX_PATH, content);
  fs.writeFileSync(INDEX_SEED_PATH, content);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const index = readJson(INDEX_PATH, 'platform package index');
  const indexPackage = (index.packages || []).find((pkg) => pkg.id === args.platformId);
  if (!indexPackage) fail(`Unknown platform package: ${args.platformId}`);

  const packageRoot = path.join(ROOT, 'platform-packages', args.platformId);
  const manifestPath = path.join(packageRoot, 'manifest.json');
  const runtimePath = path.join(packageRoot, 'runtime', 'index.json');
  const infoPath = path.join(packageRoot, 'assets', 'package-info.json');
  assertDir(packageRoot, args.platformId);
  assertFile(manifestPath, `${args.platformId}: manifest`);
  assertFile(runtimePath, `${args.platformId}: runtime`);
  assertFile(infoPath, `${args.platformId}: package-info`);

  const manifest = readJson(manifestPath, `${args.platformId} manifest`);
  const runtime = readJson(runtimePath, `${args.platformId} runtime`);
  if (manifest.id !== args.platformId || runtime.packageId !== args.platformId) {
    fail(`${args.platformId}: manifest/runtime id mismatch`);
  }
  if (args.updateIndex && manifest.version !== indexPackage.version) {
    console.warn(`${args.platformId}: updating index version ${indexPackage.version} -> ${manifest.version}`);
  }

  let stagedPackageRoot = null;
  try {
    stagedPackageRoot = createPackageStagingRoot(args.platformId);
    copyPackageSourceToStaging(packageRoot, stagedPackageRoot);

    const ui = manifest.ui || {};
    if (ui.protocol !== 'react-remote-esm-v1') fail(`${args.platformId}: ui.protocol must be react-remote-esm-v1`);
    assertFile(path.join(stagedPackageRoot, safeRelativePath(ui.entry, `${args.platformId}: ui.entry`)), `${args.platformId}: UI entry`);
    if (ui.style) {
      assertFile(path.join(stagedPackageRoot, safeRelativePath(ui.style, `${args.platformId}: ui.style`)), `${args.platformId}: UI style`);
    }

    const adapterEntry = copyAdapterIfAvailable(packageRoot, stagedPackageRoot, manifest, args.os, args.adapterBinDir);
    const helperEntries = copyCodexRuntimeHelpers(stagedPackageRoot, manifest, args.os, args.arch);
    if (manifest.adapter) {
      const cratePath = path.join(ROOT, 'crates', expectedAdapterCrateName(args.platformId), 'Cargo.toml');
      assertFile(cratePath, `${args.platformId}: adapter crate`);
      if (args.os === 'windows') {
        verifyWindowsAdapterManifestBuild(args.platformId, cratePath);
      }
    }

    const zipName = zipNameFor(args.platformId, manifest.version, args.os, args.arch, args.filenameTemplate);
    const zipPath = path.join(args.distDir, zipName);
    createZip(stagedPackageRoot, zipPath);

    const size = fs.statSync(zipPath).size;
    const checksum = sha256(zipPath);
    const firstArtifact = (indexPackage.artifacts || [])[0] || {};
    const downloadUrl = args.downloadUrl || replaceZipName(firstArtifact.downloadUrl || indexPackage.downloadUrl, zipName);
    const metadata = {
      id: args.platformId,
      platformId: manifest.platformId,
      version: manifest.version,
      packageMode: manifest.packageMode,
      installKind: manifest.installKind,
      os: args.os,
      arch: args.arch,
      zipName,
      zipPath: displayPath(zipPath),
      downloadUrl,
      downloadSizeBytes: size,
      sha256: checksum,
      adapterEntry,
      helperEntries,
      uiEntry: ui.entry,
      uiStyle: ui.style || null,
    };

    if (args.updateIndex) updateIndex(index, args.platformId, args.os, args.arch, metadata, manifest);
    if (args.metadataOut) {
      fs.mkdirSync(path.dirname(args.metadataOut), { recursive: true });
      fs.writeFileSync(args.metadataOut, `${JSON.stringify(metadata, null, 2)}\n`);
    }
    console.log(JSON.stringify(metadata, null, 2));
  } finally {
    if (stagedPackageRoot) {
      fs.rmSync(stagedPackageRoot, { recursive: true, force: true });
    }
  }
}

main();
