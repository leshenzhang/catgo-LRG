import { describe, expect, it } from 'vitest'
import { create_trajectory_bond_cache } from '$lib/structure/trajectory-bond-cache.svelte'

// Seed the private cache via the public surface: directly poke through a
// minimal cast — the class stores Map<idx, conn[]>; we only test invalidate.
function seed(cache: ReturnType<typeof create_trajectory_bond_cache>, idx: number) {
  // @ts-expect-error reach into private cache for a focused unit test
  cache.cache.set(idx, [{ site_idx_1: 0, site_idx_2: 1, strength: 1, jimage: [0, 0, 0] }])
  // @ts-expect-error
  cache.version++
}

function gen(cache: ReturnType<typeof create_trajectory_bond_cache>): number {
  // @ts-expect-error private generation counter, focused unit test
  return cache.generation as number
}
function mark_inflight(cache: ReturnType<typeof create_trajectory_bond_cache>, idx: number) {
  // @ts-expect-error private inflight set, focused unit test
  cache.inflight.add(idx)
}
function is_inflight(cache: ReturnType<typeof create_trajectory_bond_cache>, idx: number): boolean {
  // @ts-expect-error private inflight set, focused unit test
  return cache.inflight.has(idx)
}

describe('TrajectoryBondCache.invalidate', () => {
  it('drops one frame, clears its inflight flag, and bumps version + generation so an inflight result is voided', () => {
    const c = create_trajectory_bond_cache()
    seed(c, 3)
    seed(c, 4)
    mark_inflight(c, 3)
    const v0 = c.version
    const g0 = gen(c)
    c.invalidate(3)
    expect(c.get(3)).toBeNull()
    expect(c.get(4)).not.toBeNull()
    expect(is_inflight(c, 3)).toBe(false)
    expect(c.version).toBeGreaterThan(v0)
    expect(gen(c)).toBeGreaterThan(g0)
  })

  it('invalidate_all drops every frame and bumps generation', () => {
    const c = create_trajectory_bond_cache()
    seed(c, 0)
    seed(c, 1)
    const g0 = gen(c)
    c.invalidate_all()
    expect(c.get(0)).toBeNull()
    expect(c.get(1)).toBeNull()
    expect(gen(c)).toBeGreaterThan(g0)
  })
})
