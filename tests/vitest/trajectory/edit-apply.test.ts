import { describe, expect, it } from 'vitest'
import { applyDisplacements, writeSitesToCacheSlice } from '$lib/trajectory/edit-apply'

type Site = { xyz: [number, number, number]; abc: [number, number, number]; species: unknown }

function site(x: number, y: number, z: number): Site {
  return { xyz: [x, y, z], abc: [x, y, z], species: [{ element: 'O', occu: 1 }] }
}

describe('applyDisplacements', () => {
  it('returns a new sites array with xyz + abc displaced for matched indices (no lattice = abc==xyz delta)', () => {
    const sites = [site(0, 0, 0), site(1, 1, 1)]
    const out = applyDisplacements(sites, new Map([[1, [0.5, 0, -0.5]]]), null)
    expect(out).not.toBe(sites) // new array
    expect(out[0]).toBe(sites[0]) // untouched site kept by reference
    expect(out[1].xyz).toEqual([1.5, 1, 0.5])
    expect(out[1].abc).toEqual([1.5, 1, 0.5]) // no inv lattice → fractional delta == cartesian delta
    expect(sites[1].xyz).toEqual([1, 1, 1]) // input not mutated
  })

  it('uses the inverse-lattice matrix to convert the cartesian delta to a fractional abc delta', () => {
    // 2x identity lattice → inv = 0.5*I → frac delta = 0.5 * cartesian delta
    const inv: [number, number, number, number, number, number, number, number, number] =
      [0.5, 0, 0, 0, 0.5, 0, 0, 0, 0.5]
    const sites = [site(0, 0, 0)]
    const out = applyDisplacements(sites, new Map([[0, [2, 4, 6]]]), inv)
    expect(out[0].xyz).toEqual([2, 4, 6])
    expect(out[0].abc).toEqual([1, 2, 3])
  })

  it('is a no-op (same array contents, new array) when displacements is empty', () => {
    const sites = [site(0, 0, 0)]
    const out = applyDisplacements(sites, new Map(), null)
    expect(out[0]).toBe(sites[0])
  })
})

describe('writeSitesToCacheSlice', () => {
  it('mirrors sites xyz into the Float32Array slice in place, up to min(len)', () => {
    const sites = [site(1, 2, 3), site(4, 5, 6)]
    const arr = new Float32Array(6)
    writeSitesToCacheSlice(arr, sites)
    expect(Array.from(arr)).toEqual([1, 2, 3, 4, 5, 6])
  })

  it('does not overflow when the array is shorter than sites*3 (supercell-extra atoms)', () => {
    const sites = [site(1, 2, 3), site(4, 5, 6)]
    const arr = new Float32Array(3)
    writeSitesToCacheSlice(arr, sites)
    expect(Array.from(arr)).toEqual([1, 2, 3])
  })
})
