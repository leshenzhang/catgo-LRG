import { describe, expect, test } from 'vitest'

import { merge_structures } from '$lib/structure/atom-manipulation'
import { get_import_position_outside } from '$lib/structure/controllers/transform-controller'
import type { AnyStructure } from '$lib/structure'

// Covers the "merge into current structure" behavior used when a database
// (OPTIMADE/PubChem) or pasted structure is imported while a structure is
// already loaded in the viewer. handle_optimade_import / handle_paste_content_import
// compose exactly these two helpers, so this locks in that contract.

function make_site(element: string, xyz: [number, number, number]) {
  return {
    species: [{ element, occu: 1 }],
    abc: [0, 0, 0] as [number, number, number],
    xyz,
    label: element,
    properties: {} as Record<string, unknown>,
  }
}

function make_structure(
  sites: ReturnType<typeof make_site>[],
  extras: Record<string, unknown> = {},
): AnyStructure {
  return { sites, ...extras } as unknown as AnyStructure
}

function merge_outside(base: AnyStructure, incoming: AnyStructure): AnyStructure {
  return merge_structures(base, incoming, get_import_position_outside(base, incoming))
}

describe('merge_structures (import into a loaded structure)', () => {
  test('keeps every atom from both the existing and imported structures', () => {
    const base = make_structure([make_site('Pt', [0, 0, 0]), make_site('Pt', [2, 0, 0])])
    const incoming = make_structure([make_site('C', [0, 0, 0]), make_site('O', [0, 0, 1])])

    const merged = merge_outside(base, incoming)

    expect(merged.sites).toHaveLength(4)
    expect(merged.sites.map((s) => s.species[0].element)).toEqual(['Pt', 'Pt', 'C', 'O'])
  })

  test('leaves the existing atoms exactly where they were', () => {
    const base = make_structure([make_site('Pt', [0, 0, 0]), make_site('Pt', [2, 0, 0])])
    const incoming = make_structure([make_site('C', [0, 0, 0])])

    const merged = merge_outside(base, incoming)

    expect(merged.sites[0].xyz).toEqual([0, 0, 0])
    expect(merged.sites[1].xyz).toEqual([2, 0, 0])
  })

  test('places the imported atoms outside (above) the existing structure', () => {
    const base = make_structure([make_site('Pt', [0, 0, 0]), make_site('Pt', [0, 0, 5])]) // max z = 5
    const incoming = make_structure([make_site('C', [0, 0, 0]), make_site('O', [0, 0, 2])])

    const merged = merge_outside(base, incoming)

    const imported_min_z = Math.min(merged.sites[2].xyz[2], merged.sites[3].xyz[2])
    expect(imported_min_z).toBeGreaterThan(5)
  })

  test('preserves the base lattice (a molecule merged onto a slab keeps the slab cell)', () => {
    const base = make_structure(
      [make_site('Pt', [1, 1, 1])],
      { lattice: { matrix: [[10, 0, 0], [0, 10, 0], [0, 0, 20]] } },
    )
    const incoming = make_structure([make_site('C', [0, 0, 0])])

    const merged = merge_outside(base, incoming) as AnyStructure & {
      lattice?: { matrix: number[][] }
    }

    expect(merged.lattice?.matrix).toEqual([[10, 0, 0], [0, 10, 0], [0, 0, 20]])
    expect(merged.sites).toHaveLength(2)
  })

  test('returns a lattice-less molecule when the base has no lattice', () => {
    const base = make_structure([make_site('C', [0, 0, 0])])
    const incoming = make_structure([make_site('O', [0, 0, 1])])

    const merged = merge_outside(base, incoming) as AnyStructure & { lattice?: unknown }

    expect(merged.lattice).toBeUndefined()
    expect(merged.sites).toHaveLength(2)
  })
})
