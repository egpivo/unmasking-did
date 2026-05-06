# Pair Baseline Score: `data/unmask_eval_mainnet_v1.db`

- This is a deterministic, conservative baseline for feasibility/ranking.
- It is **not DID verification**, and it does not alter linker semantics.

## Recommendation Policy (Semantic Floor)
- `verified` evidence → `verified_did_or_controller`
- identity claim (`did`/`ens`/…) → `identity_claim_candidate`
- `safe_owner_count > 0` → `control_link_candidate` (control relation; not same entity)
- event-only (`funded_by`/`transfer`/…) → `related_only` (never promoted to control by count alone)
- `hub_penalty` affects `entity_confidence` and caveat text only, not semantic class

## Top Candidate Pairs
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0xc18360217d8f7ab5e7c516566761ea12ce7f9d72` | rec=related_only | conf=weak | related=0.347 control=0.000 did=0.000 | funded_by=4 safe_owner=0 | span=3696924 pattern=long_span hub=0.631 | kinds={'funded_by': 4}
- `0xae7ab96520de3a18e5e111b5eaab095312d7fe84` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.341 control=0.000 did=0.000 | funded_by=189 safe_owner=0 | span=3870206 pattern=long_span hub=0.620 | kinds={'funded_by': 189}
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.339 control=0.000 did=0.000 | funded_by=199 safe_owner=0 | span=6073280 pattern=long_span hub=0.615 | kinds={'funded_by': 199}
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` | rec=related_only | conf=weak | related=0.338 control=0.000 did=0.000 | funded_by=192 safe_owner=0 | span=7967057 pattern=long_span hub=0.615 | kinds={'funded_by': 192}
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.337 control=0.000 did=0.000 | funded_by=137 safe_owner=0 | span=5990792 pattern=long_span hub=0.612 | kinds={'funded_by': 137}
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0xae7ab96520de3a18e5e111b5eaab095312d7fe84` | rec=related_only | conf=weak | related=0.336 control=0.000 did=0.000 | funded_by=95 safe_owner=0 | span=4716092 pattern=long_span hub=0.611 | kinds={'funded_by': 95}
- `0x931896a8a9313f622a2afca76d1471b97955e551` ↔ `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` | rec=control_link_candidate | conf=medium_low | related=0.336 control=0.187 did=0.000 | funded_by=22 safe_owner=2 | span=10958687 pattern=long_span hub=0.425 | kinds={'funded_by': 22, 'safe_owner': 2}
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0xae7ab96520de3a18e5e111b5eaab095312d7fe84` | rec=related_only | conf=weak | related=0.334 control=0.000 did=0.000 | funded_by=105 safe_owner=0 | span=5939778 pattern=long_span hub=0.608 | kinds={'funded_by': 105}
- `0x1f9840a85d5af5bf1d1762f925bdaddc4201f984` ↔ `0xae7ab96520de3a18e5e111b5eaab095312d7fe84` | rec=related_only | conf=weak | related=0.329 control=0.000 did=0.000 | funded_by=4 safe_owner=0 | span=4386902 pattern=long_span hub=0.598 | kinds={'funded_by': 4}
- `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.320 control=0.105 did=0.000 | funded_by=9 safe_owner=1 | span=7023245 pattern=long_span hub=0.477 | kinds={'funded_by': 9, 'safe_owner': 1}

## Summary Diagnostics
- recommendation distribution: {'insufficient_evidence': 66, 'related_only': 117, 'control_link_candidate': 7}
- evidence kind pair coverage: {'funded_by': 124, 'safe_owner': 7}
- pairs with no evidence: 66
- pairs with funded_by: 124
- pairs with funded_by only: 117
- pairs with safe_owner present: 7
- pairs with DID/identity evidence present: 0
- pairs with verified DID evidence present: 0
- insufficient_evidence with zero evidence: 66
- insufficient_evidence with nonzero evidence: 0
- safe_owner present → control_link_candidate: 7
- safe_owner present but not control_link_candidate: 0
- funded_by-only pairs as related_only: 117
- max did_score (dataset): 0.0
- hub_penalty summary: {'count': 190, 'min': 0.2789, 'p25': 0.3705, 'median': 0.4954, 'p75': 1.0, 'max': 1.0, 'pairs_penalty_lt_0_5': 97, 'pairs_penalty_lt_0_75': 124}
- recommendation_policy: semantic_floor_verified_identity_control_event
- caveat: Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.

## Temporal Coverage
- temporal patterns: {'unknown': 66, 'long_span': 123, 'short_burst': 1}
- block_span buckets: {'unknown': 66, '2_long_span_gt_100': 123, '1_short_burst_1_to_100': 1}

## Coverage Caveat
- Event-only evidence (`funded_by`/`transfer`) supports relatedness ranking only.
- No DID evidence means `did_score` remains 0 by design.
- Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.


## Top Related Only Pairs
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0xc18360217d8f7ab5e7c516566761ea12ce7f9d72` | rec=related_only | conf=weak | related=0.347 control=0.000 did=0.000 | funded_by=4 safe_owner=0 | span=3696924 pattern=long_span hub=0.631 | kinds={'funded_by': 4}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.631) lowers confidence; does not change evidence class.
- `0xae7ab96520de3a18e5e111b5eaab095312d7fe84` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.341 control=0.000 did=0.000 | funded_by=189 safe_owner=0 | span=3870206 pattern=long_span hub=0.620 | kinds={'funded_by': 189}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.620) lowers confidence; does not change evidence class.
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.339 control=0.000 did=0.000 | funded_by=199 safe_owner=0 | span=6073280 pattern=long_span hub=0.615 | kinds={'funded_by': 199}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.615) lowers confidence; does not change evidence class.
- `0x253553366da8546fc250f225fe3d25d0c782303b` ↔ `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` | rec=related_only | conf=weak | related=0.338 control=0.000 did=0.000 | funded_by=192 safe_owner=0 | span=7967057 pattern=long_span hub=0.615 | kinds={'funded_by': 192}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.615) lowers confidence; does not change evidence class.
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=related_only | conf=weak | related=0.337 control=0.000 did=0.000 | funded_by=137 safe_owner=0 | span=5990792 pattern=long_span hub=0.612 | kinds={'funded_by': 137}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.612) lowers confidence; does not change evidence class.

## Top Control Link Candidate Pairs
- `0xd1f5d2a59ef194ae00f27b2da6599ce207dde7cb` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.258 control=0.218 did=0.000 | funded_by=2 safe_owner=2 | span=5170985 pattern=long_span hub=0.495 | kinds={'funded_by': 2, 'safe_owner': 2}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.495) lowers confidence; does not change evidence class.
- `0x931896a8a9313f622a2afca76d1471b97955e551` ↔ `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` | rec=control_link_candidate | conf=medium_low | related=0.336 control=0.187 did=0.000 | funded_by=22 safe_owner=2 | span=10958687 pattern=long_span hub=0.425 | kinds={'funded_by': 22, 'safe_owner': 2}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.425) lowers confidence; does not change evidence class.
- `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.320 control=0.105 did=0.000 | funded_by=9 safe_owner=1 | span=7023245 pattern=long_span hub=0.477 | kinds={'funded_by': 9, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.477) lowers confidence; does not change evidence class.
- `0x19e50fa5623895d5a2976693eaff5c2f879510ed` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.120 control=0.101 did=0.000 | funded_by=1 safe_owner=1 | span=7554474 pattern=long_span hub=0.460 | kinds={'funded_by': 1, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.460) lowers confidence; does not change evidence class.
- `0x931896a8a9313f622a2afca76d1471b97955e551` ↔ `0xd1f5d2a59ef194ae00f27b2da6599ce207dde7cb` | rec=control_link_candidate | conf=medium_low | related=0.284 control=0.093 did=0.000 | funded_by=16 safe_owner=1 | span=10958718 pattern=long_span hub=0.424 | kinds={'funded_by': 16, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.424) lowers confidence; does not change evidence class.

## Insufficient Evidence With Zero Evidence
- `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` ↔ `0x19e50fa5623895d5a2976693eaff5c2f879510ed` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` ↔ `0x253553366da8546fc250f225fe3d25d0c782303b` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` ↔ `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` ↔ `0x2e59a20f205bb85a89c53f1936454680651e618e` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.
- `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` ↔ `0x323a76393544d5ecca80cd6ef2a560c6a395b7e3` | rec=insufficient_evidence | conf=none | related=0.000 control=0.000 did=0.000 | funded_by=0 safe_owner=0 | span=unknown pattern=unknown hub=1.000 | kinds={}
  - caveat: No usable evidence rows for this pair.

## Insufficient Evidence With Nonzero Evidence
- none

## Top Hub-Penalized Pairs
- `0x1a9c8182c09f50c8318d769245bea52c32be35bc` ↔ `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` | rec=related_only | conf=weak | related=0.039 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=2286850 pattern=long_span hub=0.279 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.279) lowers confidence; does not change evidence class.
- `0x1f9840a85d5af5bf1d1762f925bdaddc4201f984` ↔ `0xd28b432f06cb64692379758b88b5fcdfc4f56922` | rec=related_only | conf=weak | related=0.039 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=8803990 pattern=long_span hub=0.279 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.279) lowers confidence; does not change evidence class.
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0x3e40d73eb977dc6a537af587d48316fee66e9c8c` | rec=related_only | conf=weak | related=0.039 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=1796493 pattern=long_span hub=0.279 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.279) lowers confidence; does not change evidence class.
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0x4f2083f5fbede34c2714affb3105539775f7fe64` | rec=related_only | conf=weak | related=0.039 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=6711381 pattern=long_span hub=0.279 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.279) lowers confidence; does not change evidence class.
- `0x283af0b28c62c092c9727f1ee09c02ca627eb7f5` ↔ `0x931896a8a9313f622a2afca76d1471b97955e551` | rec=related_only | conf=weak | related=0.039 control=0.000 did=0.000 | funded_by=1 safe_owner=0 | span=10828009 pattern=long_span hub=0.279 | kinds={'funded_by': 1}
  - caveat: Event/interaction evidence only; not proof of common ownership or control. Hub-like pattern (hub_penalty=0.279) lowers confidence; does not change evidence class.

## Top Safe Owner Present Pairs
- `0xd1f5d2a59ef194ae00f27b2da6599ce207dde7cb` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.258 control=0.218 did=0.000 | funded_by=2 safe_owner=2 | span=5170985 pattern=long_span hub=0.495 | kinds={'funded_by': 2, 'safe_owner': 2}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.495) lowers confidence; does not change evidence class.
- `0x931896a8a9313f622a2afca76d1471b97955e551` ↔ `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` | rec=control_link_candidate | conf=medium_low | related=0.336 control=0.187 did=0.000 | funded_by=22 safe_owner=2 | span=10958687 pattern=long_span hub=0.425 | kinds={'funded_by': 22, 'safe_owner': 2}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.425) lowers confidence; does not change evidence class.
- `0xc23da3ca9300571b9cf43298228353cbb3e1b4c0` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.320 control=0.105 did=0.000 | funded_by=9 safe_owner=1 | span=7023245 pattern=long_span hub=0.477 | kinds={'funded_by': 9, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.477) lowers confidence; does not change evidence class.
- `0x19e50fa5623895d5a2976693eaff5c2f879510ed` ↔ `0xde21f729137c5af1b01d73af1dc21effa2b8a0d6` | rec=control_link_candidate | conf=medium_low | related=0.120 control=0.101 did=0.000 | funded_by=1 safe_owner=1 | span=7554474 pattern=long_span hub=0.460 | kinds={'funded_by': 1, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.460) lowers confidence; does not change evidence class.
- `0x931896a8a9313f622a2afca76d1471b97955e551` ↔ `0xd1f5d2a59ef194ae00f27b2da6599ce207dde7cb` | rec=control_link_candidate | conf=medium_low | related=0.284 control=0.093 did=0.000 | funded_by=16 safe_owner=1 | span=10958718 pattern=long_span hub=0.424 | kinds={'funded_by': 16, 'safe_owner': 1}
  - caveat: safe_owner is a control relation, not proof of same human/entity. Hub-like pattern (hub_penalty=0.424) lowers confidence; does not change evidence class.
## Gold Diagnostics
- matched gold pairs: 190
- counts by gold label: {'different_control': 147, 'uncertain': 43}
- counts by recommendation on gold: {'insufficient_evidence': 66, 'related_only': 117, 'control_link_candidate': 7}
- caveat: Small/curated gold sets; diagnostics are directional only, not statistically definitive.
