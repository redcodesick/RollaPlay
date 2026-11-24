import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

const [tag = '', notesFile = '', pubDate = '', outputFile = 'latest.json'] = process.argv.slice(2)
const repository = process.env.GITHUB_REPOSITORY ?? ''
const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag)

if (!match) throw new Error(`Expected a stable release tag, received "${tag}".`)
if (!/^[^/\s]+\/[^/\s]+$/.test(repository)) throw new Error('GITHUB_REPOSITORY is not set.')

const version = `${match[1]}.${match[2]}.${match[3]}`
if (Number.isNaN(Date.parse(pubDate))) throw new Error('Release publication date is missing or invalid.')

const notes = readFileSync(resolve(notesFile), 'utf8').trim()
if (!notes) throw new Error('Release notes must be read from the published GitHub Release body.')

function findOne(directory, predicate, label) {
  const matches = readdirSync(resolve(directory), { withFileTypes: true })
    .filter((entry) => entry.isFile() && predicate(entry.name))
    .map((entry) => entry.name)
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${label} for ${version}, found ${matches.length}: ${matches.join(', ') || 'none'}.`)
  }
  return matches[0]
}

function readSignature(directory, artifactName) {
  const path = resolve(directory, `${artifactName}.sig`)
  const signature = readFileSync(path, 'utf8').trim()
  if (!/^[A-Za-z0-9+/=]+$/.test(signature) || signature.length < 100) {
    throw new Error(`The signature for ${artifactName} is missing or malformed.`)
  }
  return signature
}

const nsisDir = 'src-tauri/target/release/bundle/nsis'
const msiDir = 'src-tauri/target/release/bundle/msi'
const nsisName = findOne(nsisDir, (name) => name === `RollaPlay_${version}_x64-setup.exe`, 'NSIS installer')
const msiName = findOne(msiDir, (name) => name === `RollaPlay_${version}_x64_en-US.msi`, 'MSI installer')
const nsisSignature = readSignature(nsisDir, nsisName)
const msiSignature = readSignature(msiDir, msiName)

const base = `https://github.com/${repository}/releases/download/${tag}`
const nsisPlatform = { url: `${base}/${nsisName}`, signature: nsisSignature }
const manifest = {
  version,
  notes,
  pub_date: new Date(pubDate).toISOString(),
  platforms: {
    'windows-x86_64-nsis': nsisPlatform,
    'windows-x86_64-msi': { url: `${base}/${msiName}`, signature: msiSignature },
    'windows-x86_64': nsisPlatform,
  },
}

writeFileSync(resolve(outputFile), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
console.log(`Wrote ${outputFile} for ${version} from signed build artifacts.`)
