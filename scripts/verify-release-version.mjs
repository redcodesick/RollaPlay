import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? ''
const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag)

if (!match) {
  throw new Error(`Release tag must be a stable semantic version tag such as v0.3.0; received "${tag}".`)
}

const version = `${match[1]}.${match[2]}.${match[3]}`
const packageVersion = JSON.parse(readFileSync(resolve('package.json'), 'utf8')).version
const tauriVersion = JSON.parse(readFileSync(resolve('src-tauri/tauri.conf.json'), 'utf8')).version
const cargoToml = readFileSync(resolve('src-tauri/Cargo.toml'), 'utf8')
const packageBlock = /^\[package\]([\s\S]*?)(?=^\[)/m.exec(cargoToml)?.[1] ?? ''
const cargoVersion = /(?:^|\n)version\s*=\s*"([^"]+)"/.exec(packageBlock)?.[1] ?? ''

const declared = {
  'package.json': packageVersion,
  'src-tauri/tauri.conf.json': tauriVersion,
  'src-tauri/Cargo.toml': cargoVersion,
}

const mismatches = Object.entries(declared).filter(([, value]) => value !== version)
if (mismatches.length) {
  const details = mismatches.map(([file, value]) => `${file}=${value || '(missing)'}`).join(', ')
  throw new Error(`Release tag v${version} does not match ${details}. Update every version declaration before tagging.`)
}

console.log(`Release version verified: v${version}`)
