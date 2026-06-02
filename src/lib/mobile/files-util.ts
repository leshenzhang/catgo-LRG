/**
 * files-util.ts — small pure helpers for the mobile SFTP file browser
 * ({@link MobileFiles} / {@link MobileFileViewer}). Kept dependency-free and
 * side-effect-free so it is trivially testable and shared by both components.
 */

/** Human-readable byte size, e.g. `1.2 KB`, `3.4 MB`. */
export function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return `—`
  if (bytes < 1024) return `${bytes} B`
  const units = [`KB`, `MB`, `GB`, `TB`]
  let val = bytes / 1024
  let i = 0
  while (val >= 1024 && i < units.length - 1) {
    val /= 1024
    i += 1
  }
  return `${val.toFixed(val < 10 ? 1 : 0)} ${units[i]}`
}

/** POSIX-join `base` + `name`, collapsing a trailing slash on `base`. */
export function joinPath(base: string, name: string): string {
  if (base === `/`) return `/${name}`
  return `${base.replace(/\/+$/, ``)}/${name}`
}

/** Parent directory of a POSIX path (root stays root). */
export function parentPath(path: string): string {
  if (path === `/` || path === ``) return `/`
  const trimmed = path.replace(/\/+$/, ``)
  const idx = trimmed.lastIndexOf(`/`)
  if (idx <= 0) return `/`
  return trimmed.slice(0, idx)
}

/**
 * Heuristic: is this filename obviously a binary we should NOT try to render as
 * text? Extension-based; the viewer also falls back to a NUL-byte check on the
 * decoded content for extension-less binaries.
 */
const BINARY_EXTS = new Set([
  `png`, `jpg`, `jpeg`, `gif`, `bmp`, `webp`, `ico`, `tiff`, `svg`,
  `pdf`, `zip`, `gz`, `tgz`, `bz2`, `xz`, `7z`, `rar`, `tar`,
  `so`, `dll`, `dylib`, `exe`, `bin`,
  `wav`, `mp3`, `mp4`, `mov`, `avi`, `mkv`, `flac`, `ogg`,
  `pyc`, `pyo`, `npy`, `npz`, `h5`, `hdf5`, `pt`, `pth`, `ckpt`,
  `db`, `sqlite`, `pickle`, `pkl`, `parquet`, `feather`, `wasm`,
  `chgcar`, `wavecar`, `vasprun`,
])

export function isBinaryName(name: string): boolean {
  const dot = name.lastIndexOf(`.`)
  if (dot < 0) return false
  const ext = name.slice(dot + 1).toLowerCase()
  return BINARY_EXTS.has(ext)
}

// Structure-format files that should open in the 3D editor (by extension OR by
// well-known VASP basenames that carry no extension).
const STRUCTURE_EXTS = new Set([
  `cif`, `poscar`, `contcar`, `vasp`, `xyz`, `extxyz`, `cube`, `cub`,
  `lammps`, `data`, `pdb`, `mol`, `mol2`, `xsf`, `res`, `gen`,
])
const STRUCTURE_BASENAMES = new Set([`poscar`, `contcar`, `xdatcar`])

export function isStructureName(name: string): boolean {
  const base = name.toLowerCase()
  if (STRUCTURE_BASENAMES.has(base)) return true
  const dot = base.lastIndexOf(`.`)
  if (dot < 0) return false
  return STRUCTURE_EXTS.has(base.slice(dot + 1))
}
