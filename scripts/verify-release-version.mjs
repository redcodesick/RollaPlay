import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? ''
const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag)

if (!match) {
  throw new Error(`Release tag must be a stable semantic version tag such as v0.3.0; received "${tag}".`)
}

const version = `${match[1]}.${match[2]}.${match[3]}`
const read = (path) => readFileSync(resolve(path), 'utf8')

const packageVersion = JSON.parse(read('package.json')).version
const tauriVersion = JSON.parse(read('src-tauri/tauri.conf.json')).version

const cargoToml = read('src-tauri/Cargo.toml')
const packageBlock = /^\[package\]([\s\S]*?)(?=^\[)/m.exec(cargoToml)?.[1] ?? ''
const cargoVersion = /(?:^|\n)version\s*=\s*"([^"]+)"/.exec(packageBlock)?.[1] ?? ''

const cargoLock = read('src-tauri/Cargo.lock').replace(/\r\n/g, '\n')
const lockPackage = cargoLock
  .split(/^\[\[package\]\]$/m)
  .find((block) => /(?:^|\n)name\s*=\s*"rollaplay"/.test(block)) ?? ''
const cargoLockVersion = /(?:^|\n)version\s*=\s*"([^"]+)"/.exec(lockPackage)?.[1] ?? ''

const versionSource = read('src/version.ts')
const frontendVersion = /export const APP_VERSION\s*=\s*['"]([^'"]+)['"]/.exec(versionSource)?.[1] ?? ''

const declared = {
  'package.json': packageVersion,
  'src-tauri/tauri.conf.json': tauriVersion,
  'src-tauri/Cargo.toml': cargoVersion,
  'src-tauri/Cargo.lock': cargoLockVersion,
  'src/version.ts': frontendVersion,
}

const mismatches = Object.entries(declared).filter(([, value]) => value !== version)
if (mismatches.length) {
  const details = mismatches.map(([file, value]) => `${file}=${value || '(missing)'}`).join(', ')
  throw new Error(`Release tag v${version} does not match ${details}. Update every version declaration before tagging.`)
}

console.log(`Release version verified: v${version}`)
