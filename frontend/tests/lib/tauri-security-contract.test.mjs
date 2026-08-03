// Ported and adapted from henryvn27/meetily_improved (commit ad1a9bb).
// Policy difference from the fork: this tree allowlists every command registered
// in `tauri::generate_handler!` (registry-based), not only commands with a
// shipped frontend caller. The test enforces that the three lists stay in
// lockstep and that the capability file keeps its least-privilege shape.
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import test from 'node:test';

const frontendRoot = new URL('../../', import.meta.url);

// Frontend calls that target commands which are NOT registered in
// generate_handler. They were already dead before the ACL existed (the IPC
// call fails with "command not found"). Do not grow this list: either
// register the command or remove the caller.
const KNOWN_DEAD_CALLERS = new Set([
  'api_get_auto_generate_setting',
  'builtin_ai_get_models_directory',
]);

function stringsInArray(source, marker) {
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `missing ${marker}`);
  const assignment = source.indexOf('=', start);
  const open = source.indexOf('[', assignment);
  const close = source.indexOf(']', open);
  assert.ok(assignment > start && open > assignment && close > open, `invalid array after ${marker}`);
  return new Set([...source.slice(open + 1, close).matchAll(/"([a-z][a-z0-9_]*)"/g)]
    .map((match) => match[1]));
}

function registeredCommands(libSource) {
  const marker = '.invoke_handler(tauri::generate_handler![';
  const start = libSource.indexOf(marker);
  const end = libSource.indexOf('])', start);
  assert.ok(start >= 0 && end > start, 'Rust invoke handler registration must be present');
  return new Set([...libSource.slice(start + marker.length, end)
    .matchAll(/^\s*(?:[a-zA-Z_][a-zA-Z0-9_]*::)*([a-zA-Z_][a-zA-Z0-9_]*),\s*$/gm)]
    .map((match) => match[1]));
}

async function sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async (entry) => {
    const url = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, directory);
    if (entry.isDirectory()) return sourceFiles(url);
    return /\.(?:js|jsx|mjs|ts|tsx)$/.test(entry.name) ? [url] : [];
  }));
  return nested.flat();
}

async function invokedCommands() {
  const files = await sourceFiles(new URL('src/', frontendRoot));
  const commands = new Set();
  for (const url of files) {
    const source = await readFile(url, 'utf8');
    for (const match of source.matchAll(/\binvoke(?:Tauri)?(?:<[^>]*>)?\(\s*['"`]([a-z][a-z0-9_]*)['"`]/g)) {
      commands.add(match[1]);
    }
  }
  return commands;
}

function sorted(values) {
  return [...values].sort();
}

test('manifest, permission, and Rust registration stay in lockstep', async () => {
  const [manifestSource, permissionSource, libSource] = await Promise.all([
    readFile(new URL('src-tauri/build/webview_commands.rs', frontendRoot), 'utf8'),
    readFile(new URL('src-tauri/permissions/main-window.toml', frontendRoot), 'utf8'),
    readFile(new URL('src-tauri/src/lib.rs', frontendRoot), 'utf8'),
  ]);
  const manifestCommands = stringsInArray(manifestSource, 'WEBVIEW_COMMANDS');
  const permissionCommands = stringsInArray(permissionSource, 'commands.allow');
  const registered = registeredCommands(libSource);

  assert.deepEqual(sorted(manifestCommands), sorted(registered),
    'build/webview_commands.rs must list exactly the commands registered in generate_handler!');
  assert.deepEqual(sorted(permissionCommands), sorted(registered),
    'permissions/main-window.toml must list exactly the commands registered in generate_handler!');
});

test('every frontend invoke() targets an allowlisted command', async () => {
  const libSource = await readFile(new URL('src-tauri/src/lib.rs', frontendRoot), 'utf8');
  const registered = registeredCommands(libSource);
  const invoked = await invokedCommands();

  const unknown = sorted([...invoked].filter(
    (command) => !registered.has(command) && !KNOWN_DEAD_CALLERS.has(command),
  ));
  assert.deepEqual(unknown, [],
    'frontend invokes commands that are not registered (and therefore denied by the ACL)');
});

test('the main capability grants only caller-proven core and plugin operations', async () => {
  const [configSource, capabilitySource, packageSource, cargoSource] = await Promise.all([
    readFile(new URL('src-tauri/tauri.conf.json', frontendRoot), 'utf8'),
    readFile(new URL('src-tauri/capabilities/main.json', frontendRoot), 'utf8'),
    readFile(new URL('package.json', frontendRoot), 'utf8'),
    readFile(new URL('src-tauri/Cargo.toml', frontendRoot), 'utf8'),
  ]);
  const config = JSON.parse(configSource);
  const main = JSON.parse(capabilitySource);
  const packageJson = JSON.parse(packageSource);
  assert.deepEqual(config.app.security.capabilities, ['main']);
  assert.equal(main.identifier, 'main');

  const expectedPermissions = [
    'main-window-commands',
    'core:app:allow-version',
    'core:event:allow-listen',
    'core:event:allow-unlisten',
    'core:event:allow-emit',
    'core:path:allow-resolve-directory',
    'core:resources:allow-close',
    'store:allow-load',
    'store:allow-get',
    'store:allow-has',
    'store:allow-set',
    'store:allow-save',
    'updater:allow-check',
    'updater:allow-download',
    'updater:allow-install',
    'updater:allow-download-and-install',
    'process:allow-restart',
    'os:allow-platform',
  ];
  assert.deepEqual(sorted(main.permissions), sorted(expectedPermissions));
  assert.deepEqual(main.windows, ['main']);

  assert.equal(packageJson.dependencies?.['@tauri-apps/plugin-fs'], undefined);
  assert.equal(packageJson.devDependencies?.['@tauri-apps/plugin-fs'], undefined);
  assert.equal(packageJson.dependencies?.['@tauri-apps/plugin-notification'], undefined);
  assert.doesNotMatch(cargoSource, /tauri-plugin-fs|macos-private-api|protocol-asset/);
});

test('production CSP exposes IPC but no web network or asset-protocol surface', async () => {
  const config = JSON.parse(await readFile(new URL('src-tauri/tauri.conf.json', frontendRoot), 'utf8'));
  const { csp, devCsp, assetProtocol } = config.app.security;

  assert.equal(csp['connect-src'], "'self' ipc: http://ipc.localhost");
  assert.equal(csp['object-src'], "'none'");
  assert.equal(csp['frame-ancestors'], "'none'");
  assert.equal(assetProtocol, undefined);
  assert.equal(config.app.macOSPrivateApi, undefined);
  assert.doesNotMatch(JSON.stringify(csp), /api\.ollama\.ai|localhost:(?:5167|8178)|asset:/);
  assert.match(devCsp['connect-src'], /ws:\/\/localhost:3118/);
});
