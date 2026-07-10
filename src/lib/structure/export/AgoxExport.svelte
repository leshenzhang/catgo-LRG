<script lang="ts">
  import type { AnyStructure } from '$lib'
  import { Icon } from '$lib'
  import {
    structure_to_poscar, structure_to_xyz,
    type StructureData,
  } from '$lib/structure/export/offline-serialize'

  let {
    structure = undefined,
    prefix = $bindable('agox_gcmc'),
    generated_output = $bindable<Record<string, string>>({}),
    generation_error = $bindable<string | null>(null),
    active_file = $bindable(''),
  }: {
    structure?: AnyStructure
    prefix?: string
    generated_output?: Record<string, string>
    generation_error?: string | null
    active_file?: string
  } = $props()

  // ====== Search settings ======
  let search_mode = $state<'gcmc' | 'gcbh' | 'bh'>('gcmc')
  let n_iterations = $state(1000)
  let seed = $state(42)

  // ====== Grand-canonical ensemble ======
  let gc_species = $state('O')
  let chemical_potential = $state(-0.3)
  let mu_reference = $state<'half_o2' | 'half_h2' | 'h2o_l' | 'custom'>('half_o2')
  let temperature = $state(500)
  let n_min = $state(0)
  let n_max = $state(16)

  // ====== MC moves ======
  let w_insert = $state(0.25)
  let w_delete = $state(0.25)
  let w_rattle = $state(0.5)
  let rattle_amplitude = $state(0.5)

  // ====== Confinement cell ======
  let z_offset = $state(1.0)
  let box_height = $state(6.0)

  // ====== Calculator ======
  let calc_backend = $state<'uma' | 'mace' | 'emt' | 'vasp' | 'cp2k'>('uma')
  let mlip_model = $state('uma-s-1p1')
  let device = $state<'cuda' | 'cpu'>('cuda')
  let relax_candidates = $state(true)
  let relax_fmax = $state(0.05)
  let relax_steps = $state(100)

  // ====== Output ======
  let keep_lowest = $state(10)

  const MU_REF_LABEL: Record<string, string> = {
    half_o2: '1/2 E[O2]',
    half_h2: '1/2 E[H2]',
    h2o_l: 'E[H2O(l)]',
    custom: 'custom reference',
  }

  // ====== Presets ======
  type AgoxPreset = 'oxidation' | 'hydroxylation' | 'hydrogenation' | 'cluster'

  function apply_preset(preset: AgoxPreset) {
    generated_output = {}
    switch (preset) {
      case 'oxidation':
        search_mode = 'gcmc'; gc_species = 'O'; mu_reference = 'half_o2'
        chemical_potential = -0.3; temperature = 500; n_min = 0; n_max = 16
        break
      case 'hydroxylation':
        search_mode = 'gcmc'; gc_species = 'OH'; mu_reference = 'h2o_l'
        chemical_potential = -0.5; temperature = 400; n_min = 0; n_max = 12
        break
      case 'hydrogenation':
        search_mode = 'gcmc'; gc_species = 'H'; mu_reference = 'half_h2'
        chemical_potential = -0.2; temperature = 300; n_min = 0; n_max = 24
        break
      case 'cluster':
        search_mode = 'gcbh'; gc_species = 'Pt'; mu_reference = 'custom'
        chemical_potential = -5.5; temperature = 800; n_min = 1; n_max = 40
        break
    }
  }

  const has_lattice = $derived(
    structure && `lattice` in (structure as any) && !!(structure as any)?.lattice,
  )

  function generate() {
    generation_error = null
    try {
      if (!structure) {
        generation_error = 'No structure loaded. Load a template slab/cluster first.'
        return
      }
      const w_sum = w_insert + w_delete + w_rattle
      if (w_sum <= 0) {
        generation_error = 'MC move weights must sum to a positive value.'
        return
      }
      if (n_min > n_max) {
        generation_error = 'N min must not exceed N max.'
        return
      }

      const files: Record<string, string> = {}

      // ── Template structure (offline serialization, no backend) ──
      const template_file = has_lattice ? 'POSCAR' : 'template.xyz'
      files[template_file] = has_lattice
        ? structure_to_poscar(structure as unknown as StructureData, 'CatGO AGOX+GCMC template')
        : structure_to_xyz(structure as unknown as StructureData, 'CatGO AGOX+GCMC template')

      // ── AGOX run script ──
      const mode_label = { gcmc: 'grand-canonical MC', gcbh: 'grand-canonical basin hopping', bh: 'basin hopping (canonical)' }[search_mode]
      const norm = (w: number) => (w / w_sum).toFixed(3)

      let calc_block = ''
      if (calc_backend === 'uma') {
        calc_block = `from fairchem.core import pretrained_mlip, FAIRChemCalculator
predictor = pretrained_mlip.get_predict_unit('${mlip_model}', device='${device}')
calc = FAIRChemCalculator(predictor, task_name='oc20')`
      } else if (calc_backend === 'mace') {
        calc_block = `from mace.calculators import mace_mp
calc = mace_mp(model='${mlip_model}', device='${device}', default_dtype='float64')`
      } else if (calc_backend === 'emt') {
        calc_block = `from ase.calculators.emt import EMT
calc = EMT()  # test-only potential`
      } else if (calc_backend === 'vasp') {
        calc_block = `from ase.calculators.vasp import Vasp
calc = Vasp(xc='PBE', encut=400, kpts=(2, 2, 1), ismear=1, sigma=0.1,
            ediff=1e-5, lreal='Auto', ncore=4)`
      } else {
        calc_block = `from ase.calculators.cp2k import CP2K
calc = CP2K(basis_set='DZVP-MOLOPT-SR-GTH', pseudo_potential='GTH-PBE',
            cutoff=400 * 27.2114, xc='PBE')`
      }

      const py = `#!/usr/bin/env python3
# CatGO export — AGOX + GCMC structure search (${mode_label})
# Engine: AGOX (https://agox.gitlab.io)
# GC species: ${gc_species} | mu = ${chemical_potential} eV vs ${MU_REF_LABEL[mu_reference]} | T = ${temperature} K
import numpy as np
from ase.io import read

from agox import AGOX
from agox.databases import Database
from agox.environments import Environment
from agox.samplers import MetropolisSampler
from agox.generators import RandomGenerator, RattleGenerator
from agox.evaluators import LocalOptimizationEvaluator

SEED = ${seed}
np.random.seed(SEED)

# ── Template ────────────────────────────────────────────────────────────
template = read('${template_file}')

# ── Confinement cell: GC species inserted ${z_offset} A above the surface,
#    within a ${box_height} A tall box spanning the full xy cell ──────────
z_top = template.positions[:, 2].max()
confinement_corner = np.array([0.0, 0.0, z_top + ${z_offset}])
confinement_cell = np.array(template.cell.copy())
confinement_cell[2, 2] = ${box_height}

environment = Environment(
    template=template,
    symbols='${gc_species}' * ${n_max},          # grand-canonical species pool (N_max = ${n_max})
    confinement_cell=confinement_cell,
    confinement_corner=confinement_corner,
)

# ── Calculator (${calc_backend.toUpperCase()}) ──
${calc_block}

# ── Database ────────────────────────────────────────────────────────────
database = Database(filename='${prefix}.db', order=6)

# ── Grand-canonical Metropolis sampler ──────────────────────────────────
# Acceptance: min(1, exp(-(dE - mu * dN) / kT)), mu = ${chemical_potential} eV vs ${MU_REF_LABEL[mu_reference]}
sampler = MetropolisSampler(
    temperature=${temperature},                  # K
    chemical_potential={'${gc_species}': ${chemical_potential}},
    n_atoms_range=(${n_min}, ${n_max}),
    order=4,
)

# ── MC move generators (normalized weights) ─────────────────────────────
random_generator = RandomGenerator(**environment.get_confinement())   # insertion
rattle_generator = RattleGenerator(
    **environment.get_confinement(),
    rattle_amplitude=${rattle_amplitude},        # A
)
generators = [random_generator, rattle_generator]
# insert=${norm(w_insert)} / delete=${norm(w_delete)} / rattle=${norm(w_rattle)}
generator_weights = [${norm(w_insert)} + ${norm(w_delete)}, ${norm(w_rattle)}]

# ── Evaluator${relax_candidates ? ': local relaxation of each MC candidate' : ': single-point only'} ──
evaluator = LocalOptimizationEvaluator(
    calc,
    gets={'get_key': 'candidates'},
    ${relax_candidates ? `optimizer_run_kwargs={'fmax': ${relax_fmax}, 'steps': ${relax_steps}},` : `optimizer_run_kwargs={'steps': 0},  # single-point`}
    store_trajectory=False,
    order=5,
)

# ── Assemble & run ──────────────────────────────────────────────────────
agox = AGOX(
    database,
    environment,
    sampler,
    *generators,
    evaluator,
    seed=SEED,
)
agox.run(N_iterations=${n_iterations})

# ── Harvest: ${keep_lowest} lowest grand-canonical energy structures ──
from ase.io import write
candidates = database.get_all_candidates()
omega = [c.get_potential_energy() - ${chemical_potential} * sum(
    1 for a in c if a.symbol in '${gc_species}') for c in candidates]
best = [c for _, c in sorted(zip(omega, candidates), key=lambda t: t[0])][:${keep_lowest}]
write('${prefix}_best.traj', best)
print(f'Done: {len(candidates)} structures sampled, {len(best)} kept -> ${prefix}_best.traj')
`
      files[`${prefix}.py`] = py

      // ── Run script ──
      let run = `#!/bin/bash\n`
      run += `# CatGO AGOX+GCMC — ${mode_label}\n`
      run += `# species=${gc_species}, mu=${chemical_potential} eV vs ${MU_REF_LABEL[mu_reference]}, T=${temperature} K, ${n_iterations} iterations\n\n`
      run += `cd "$(dirname "$0")"\n\n`
      run += `python ${prefix}.py > ${prefix}.log 2>&1\n`
      run += `echo "AGOX+GCMC finished. Database: ${prefix}.db, best structures: ${prefix}_best.traj"\n`
      files[`run_${prefix}.sh`] = run

      generated_output = files
      active_file = `${prefix}.py`
    } catch (e) {
      generation_error = e instanceof Error ? e.message : `Failed to generate AGOX input`
    }
  }
</script>

<div class="section-content calc-section">
  <div style="font-size: 0.78em; color: var(--info-color, #5b9bd5); padding: 0.4em 0.5em; margin-bottom: 0.5em; background: light-dark(rgba(91,155,213,0.08), rgba(91,155,213,0.1)); border-radius: 4px; border-left: 3px solid var(--info-color, #5b9bd5);">
    Generates input for <a href="https://agox.gitlab.io" target="_blank" rel="noopener" style="color: inherit; text-decoration: underline;">AGOX</a> grand-canonical Monte Carlo (GCMC) structure search.
    The current structure is exported as the fixed template; GC species are inserted/deleted/rattled inside a confinement cell above it.
  </div>

  <!-- Preset buttons -->
  <div style="display: flex; gap: 0.25rem; margin-bottom: 0.5em; flex-wrap: wrap;">
    <button class="preset-btn" onclick={() => apply_preset('oxidation')} title="Surface oxidation: GCMC over O coverage vs 1/2 E[O2]">O coverage</button>
    <button class="preset-btn" onclick={() => apply_preset('hydroxylation')} title="Hydroxylation: GCMC over OH vs H2O(l)">Hydroxylation</button>
    <button class="preset-btn" onclick={() => apply_preset('hydrogenation')} title="H coverage: GCMC over H vs 1/2 E[H2]">H coverage</button>
    <button class="preset-btn preset-cluster" onclick={() => apply_preset('cluster')} title="Grand-canonical basin hopping cluster growth">Cluster growth</button>
  </div>

  <div class="param-row">
    <span>Prefix</span>
    <input type="text" bind:value={prefix} class="text-input" />
  </div>
  <div class="param-row">
    <span>Search mode</span>
    <select bind:value={search_mode} onchange={() => generated_output = {}}>
      <option value="gcmc">GCMC (grand-canonical MC)</option>
      <option value="gcbh">GC basin hopping</option>
      <option value="bh">Basin hopping (canonical)</option>
    </select>
  </div>
  <div class="param-row">
    <span>Iterations <span class="param-help" title="Number of AGOX search iterations (MC steps)">?</span></span>
    <input type="number" min="100" max="1000000" step="100" bind:value={n_iterations} />
  </div>
  <div class="param-row">
    <span>Random seed</span>
    <input type="number" min="0" step="1" bind:value={seed} />
  </div>

  <!-- Grand-canonical ensemble -->
  <details class="advanced-details" open>
    <summary>Grand-Canonical Ensemble</summary>
    <div class="param-row">
      <span>GC species <span class="param-help" title="Species inserted/deleted by the grand-canonical moves (e.g. O, H, OH, Pt)">?</span></span>
      <input type="text" bind:value={gc_species} class="text-input" placeholder="O" />
    </div>
    <div class="param-row">
      <span>&mu; (eV) <span class="param-help" title="Chemical potential of the GC species relative to the selected reference. Acceptance uses dE - mu*dN.">?</span></span>
      <input type="number" min="-10" max="5" step="0.05" bind:value={chemical_potential} />
    </div>
    <div class="param-row">
      <span>&mu; reference <span class="param-help" title="Reference state the chemical potential is measured against">?</span></span>
      <select bind:value={mu_reference}>
        <option value="half_o2">1/2 E[O&#8322;]</option>
        <option value="half_h2">1/2 E[H&#8322;]</option>
        <option value="h2o_l">E[H&#8322;O(l)]</option>
        <option value="custom">Custom</option>
      </select>
    </div>
    <div class="param-row">
      <span>Temperature (K) <span class="param-help" title="Metropolis acceptance temperature">?</span></span>
      <input type="number" min="100" max="3000" step="50" bind:value={temperature} />
    </div>
    <div class="param-row">
      <span>N min <span class="param-help" title="Minimum number of GC species in the cell">?</span></span>
      <input type="number" min="0" max="200" step="1" bind:value={n_min} />
    </div>
    <div class="param-row">
      <span>N max <span class="param-help" title="Maximum number of GC species in the cell">?</span></span>
      <input type="number" min="1" max="200" step="1" bind:value={n_max} />
    </div>
  </details>

  <!-- MC moves -->
  <details class="advanced-details" open>
    <summary>MC Moves</summary>
    <div class="param-row">
      <span>Insertion weight <span class="param-help" title="Relative probability of inserting one GC species (weights are normalized)">?</span></span>
      <input type="number" min="0" max="1" step="0.05" bind:value={w_insert} />
    </div>
    <div class="param-row">
      <span>Deletion weight <span class="param-help" title="Relative probability of deleting one GC species">?</span></span>
      <input type="number" min="0" max="1" step="0.05" bind:value={w_delete} />
    </div>
    <div class="param-row">
      <span>Rattle weight <span class="param-help" title="Relative probability of displacing (rattling) existing GC species">?</span></span>
      <input type="number" min="0" max="1" step="0.05" bind:value={w_rattle} />
    </div>
    <div class="param-row">
      <span>Rattle amplitude (&#8491;) <span class="param-help" title="Max per-atom displacement of the RattleGenerator">?</span></span>
      <input type="number" min="0.1" max="3" step="0.1" bind:value={rattle_amplitude} />
    </div>
  </details>

  <!-- Confinement cell -->
  <details class="advanced-details" open>
    <summary>Confinement Cell</summary>
    <div class="param-row">
      <span>z offset (&#8491;) <span class="param-help" title="Bottom of the insertion box, measured above the topmost template atom">?</span></span>
      <input type="number" min="0" max="10" step="0.5" bind:value={z_offset} />
    </div>
    <div class="param-row">
      <span>Box height (&#8491;) <span class="param-help" title="Height of the insertion box along z; xy spans the full cell">?</span></span>
      <input type="number" min="1" max="20" step="0.5" bind:value={box_height} />
    </div>
  </details>

  <!-- Calculator -->
  <details class="advanced-details" open>
    <summary>Calculator</summary>
    <div class="param-row">
      <span>Backend <span class="param-help" title="Energy/force engine used by AGOX. MLIPs (UMA/MACE) for fast sampling; DFT (VASP/CP2K) for refinement.">?</span></span>
      <select bind:value={calc_backend} onchange={() => generated_output = {}}>
        <option value="uma">UMA (fairchem MLIP)</option>
        <option value="mace">MACE-MP</option>
        <option value="emt">EMT (test)</option>
        <option value="vasp">VASP (ASE)</option>
        <option value="cp2k">CP2K (ASE)</option>
      </select>
    </div>
    {#if calc_backend === 'uma' || calc_backend === 'mace'}
      <div class="param-row">
        <span>Model</span>
        <input type="text" bind:value={mlip_model} class="text-input" placeholder={calc_backend === 'uma' ? 'uma-s-1p1' : 'medium-mpa-0'} />
      </div>
      <div class="param-row">
        <span>Device</span>
        <select bind:value={device}>
          <option value="cuda">cuda</option>
          <option value="cpu">cpu</option>
        </select>
      </div>
    {/if}
    <label class="checkbox-row">
      <input type="checkbox" bind:checked={relax_candidates} />
      Locally relax each MC candidate
    </label>
    {#if relax_candidates}
      <div class="param-row">
        <span>fmax (eV/&#8491;)</span>
        <input type="number" min="0.01" max="0.5" step="0.01" bind:value={relax_fmax} />
      </div>
      <div class="param-row">
        <span>Max relax steps</span>
        <input type="number" min="10" max="1000" step="10" bind:value={relax_steps} />
      </div>
    {/if}
  </details>

  <!-- Output -->
  <details class="advanced-details">
    <summary>Output</summary>
    <div class="param-row">
      <span>Keep lowest <span class="param-help" title="Number of lowest grand-canonical energy structures written to the final trajectory">?</span></span>
      <input type="number" min="1" max="100" step="1" bind:value={keep_lowest} />
    </div>
  </details>

  <div class="button-group">
    <button class="generate-btn" onclick={generate}>
      <Icon icon="Zap" style="width: 14px; height: 14px" /> Generate
    </button>
  </div>
</div>

<style>
  .calc-section { max-height: 400px; overflow-y: auto; }
  .param-row span { flex-shrink: 0; }
  .param-help {
    display: inline-flex; align-items: center; justify-content: center;
    width: 13px; height: 13px; font-size: 9px; font-weight: 700;
    border-radius: 50%; background: var(--btn-bg, light-dark(rgba(0,0,0,0.08), rgba(255,255,255,0.1)));
    color: var(--text-color-muted); cursor: help; flex-shrink: 0; margin-left: 2px;
    border: 1px solid var(--btn-bg, light-dark(rgba(0,0,0,0.12), rgba(255,255,255,0.15)));
    line-height: 1; vertical-align: middle;
  }
  .param-help:hover { background: var(--btn-bg-hover, light-dark(rgba(0,0,0,0.15), rgba(255,255,255,0.2))); color: var(--text-color); }
  .param-row input[type="number"], .param-row input[type="text"], .param-row select { width: 100px; text-align: right; flex-shrink: 0; }
  .text-input { flex: 1 !important; width: auto !important; min-width: 60px; }
  .advanced-details { background: light-dark(rgba(0,0,0,0.02), rgba(255,255,255,0.02)); border-radius: 4px; padding: 0.4em; margin: 0.5em 0; }
  .button-group { margin-top: 0.6em; }
  .generate-btn { display: flex; align-items: center; gap: 5px; padding: 5px 10px; background: var(--accent-color, #007acc); color: white; border: none; border-radius: 4px; cursor: pointer; }
  .generate-btn:hover { filter: brightness(1.1); }
  .preset-btn { padding: 2px 8px; font-size: 0.8em; background: rgba(5,150,105,0.3); border: 1px solid rgba(5,150,105,0.5); border-radius: 3px; cursor: pointer; color: #059669; white-space: nowrap; }
  .preset-btn:hover { background: rgba(5,150,105,0.5); }
  .preset-cluster { background: rgba(139,92,246,0.3); border-color: rgba(139,92,246,0.5); color: #8b5cf6; }
  .preset-cluster:hover { background: rgba(139,92,246,0.5); }
  .checkbox-row { display: flex; align-items: center; gap: 6px; font-size: 0.9em; cursor: pointer; padding: 2px 0; }
</style>
