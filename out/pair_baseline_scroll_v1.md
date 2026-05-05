# Pair Baseline Score: `data/unmask_eval_scroll_v1.db`

- This is a deterministic, conservative baseline for feasibility/ranking.
- It is **not DID verification**, and it does not alter linker semantics.

## Recommendation Policy (Semantic Floor)
- `verified` evidence → `verified_did_or_controller`
- identity claim (`did`/`ens`/…) → `identity_claim_candidate`
- `safe_owner_count > 0` → `control_link_candidate` (control relation; not same entity)
- event-only (`funded_by`/`transfer`/…) → `related_only` (never promoted to control by count alone)
- `hub_penalty` affects `entity_confidence` and caveat text only, not semantic class

## Top Candidate Pairs
- `0xd0d05390d922a2c45a70eaa4601600f236c02acc` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.220 control=0.296 did=0.000 | funded_by=1 safe_owner=3 | span=250126 pattern=long_span hub=0.448 | kinds={'funded_by': 1, 'safe_owner': 3}
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=761043 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=1011169 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
- `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=unknown pattern=unknown hub=0.387 | kinds={'safe_owner': 3}
- `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
- `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
- `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
- `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}

## Summary Diagnostics
- recommendation distribution: {'insufficient_evidence': 24, 'related_only': 2, 'control_link_candidate': 10}
- evidence kind pair coverage: {'funded_by': 3, 'safe_owner': 10}
- pairs with no evidence: 24
- pairs with funded_by: 3
- pairs with funded_by only: 2
- pairs with safe_owner present: 10
- pairs with DID/identity evidence present: 0
- pairs with verified DID evidence present: 0
- insufficient_evidence with zero evidence: 24
- insufficient_evidence with nonzero evidence: 0
- safe_owner present → control_link_candidate: 10
- safe_owner present but not control_link_candidate: 0
- funded_by-only pairs as related_only: 2
- max did_score (dataset): 0.0
- hub_penalty summary: {'count': 36, 'min': 0.3869, 'p25': 0.4479, 'median': 1.0, 'p75': 1.0, 'max': 1.0, 'pairs_penalty_lt_0_5': 10, 'pairs_penalty_lt_0_75': 12}
- recommendation_policy: semantic_floor_verified_identity_control_event
- caveat: Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.

## Temporal Coverage
- temporal patterns: {'unknown': 25, 'long_span': 5, 'same_block': 6}
- block_span buckets: {'unknown': 25, '2_long_span_gt_100': 5, '0_same_block': 6}

## Coverage Caveat
- Event-only evidence (`funded_by`/`transfer`) supports relatedness ranking only.
- No DID evidence means `did_score` remains 0 by design.
- Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.


## Top Related Only Pairs
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x73506528332becf6121f71ac9aad43646a41994c` | rec=related_only | conf=weak | related=0.088 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=2324176 pattern=long_span hub=0.631 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.631) lowers confidence; does not change evidence class.
- `0x558581b0345d986ba5bd6f04efd27e2a5b991320` ↔ `0x73506528332becf6121f71ac9aad43646a41994c` | rec=related_only | conf=weak | related=0.088 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=12987431 pattern=long_span hub=0.631 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.631) lowers confidence; does not change evidence class.

## Top Control Link Candidate Pairs
- `0xd0d05390d922a2c45a70eaa4601600f236c02acc` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.220 control=0.296 did=0.000 | funded_by=1 safe_owner=3 | span=250126 pattern=long_span hub=0.448 | kinds={'funded_by': 1, 'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.448) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=761043 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=1011169 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.

## Insufficient Evidence With Zero Evidence
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x558581b0345d986ba5bd6f04efd27e2a5b991320` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xbc72d9f10f6626271092764467983122cf15e3f4` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xcca54b0916cee2186b47e9709bedcb7041a8f761` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x558581b0345d986ba5bd6f04efd27e2a5b991320` ↔ `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x558581b0345d986ba5bd6f04efd27e2a5b991320` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.

## Insufficient Evidence With Nonzero Evidence
- none

## Top Hub-Penalized Pairs
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=761043 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=1011169 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=unknown pattern=unknown hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.

## Top Safe Owner Present Pairs
- `0xd0d05390d922a2c45a70eaa4601600f236c02acc` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.220 control=0.296 did=0.000 | funded_by=1 safe_owner=3 | span=250126 pattern=long_span hub=0.448 | kinds={'funded_by': 1, 'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.448) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=0 pattern=same_block hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=761043 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607` ↔ `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | rec=control_link_candidate | conf=medium_low | related=0.135 control=0.255 did=0.000 | funded_by=0 safe_owner=3 | span=1011169 pattern=long_span hub=0.387 | kinds={'safe_owner': 3}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.387) lowers confidence; does not change evidence class.
## Gold Diagnostics
- matched gold pairs: 36
- counts by gold label: {'same_control': 25, 'different_control': 8, 'uncertain': 3}
- counts by recommendation on gold: {'insufficient_evidence': 24, 'related_only': 2, 'control_link_candidate': 10}
- caveat: Small/curated gold sets; diagnostics are directional only, not statistically definitive.

