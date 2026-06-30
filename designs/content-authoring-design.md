# wok-engine - Content Authoring Design

Design canon for the in-editor 3D content-creation suite: an opinionated, simplified way to model, paint, rig, and animate low-poly assets inside the wok editor, without Blender. Status as of 2026-06-26: DESIGN / exploration, not yet a build track; sequenced after the scene editor (roadmap 3b/4) and folded into the Prefab view (roadmap 6), which it expands. The core paradigm (CSG), the skinning ceiling (envelopes), and normal maps are locked (2026-06-26); the only open item is which CSG kernel to depend on (pending Harrison's dependency approval). Captured here so the thinking survives; per-bite specs are written when the track is reached.

## Why

Harrison and Ryan do not yet author 3D, and Blender is a steeper climb than the project wants to take on for the bulk of its assets. The project's thesis is reinvent the workflow, not the wheel: take the established ideas (CSG, UVs, materials, skeletons, skinning, clips) and wrap them in a small, opinionated, MS-Paint-grade tool that removes most of the knobs a general DCC exposes. The labour model makes this viable - Claude builds, Harrison verifies, the result is owned forever and only maintained. The bet is do more with less: each hard subsystem has a bare-bones tier unlocked by one opinionated choice, and those choices suit a stylized low-poly look. The boolean kernel itself is the one wheel NOT to reinvent (see CSG kernel below).

## Boundary (unchanged)

The editor authors space, identity, and now asset data (geometry, material, skeleton, weights, clips, sockets). The game still owns behaviour: which clip plays when, what attaches to a socket, per-instance config - bound to a placement by id or name (see editor-design.md placement boundary). The suite produces data; it never authors the runtime state machine.

## The paradigm: CSG (the hero verb is merge)

Modeling is constructive solid geometry: place simple primitives, then combine them with real boolean union / subtract / intersect into one coherent solid. This is the established, simpler-than-Blender paradigm (think OpenSCAD or MagicaCSG): you think in whole shapes, not vertices. Merge is the core verb and the reason the tool beats Blender for this team. Direct vertex manipulation survives as a SECONDARY tool for tweaks, not the primary modeling verb.

## The bare-bones tiers (do more with less)

- Modeling / holes: real CSG booleans (union/subtract/intersect) via an existing robust kernel. Complex shapes, indents, and through-holes all come from booleans - no bespoke per-operation tooling, no hand-rolled boolean math.
- UVs: tri-planar projection on the CSG result. Booleans scramble any parametric UVs, so projection (no unwrap) is the answer; it also suits stylized texturing. Deletes unwrapping.
- Normal maps: paint a grayscale height/bump (pull/push), auto-derive the normal map. Deletes normal authoring.
- Skinning: a ladder, CAPPED at envelopes - rigid (one bone per part) to soft seams (auto-blend only the joint-ring vertices) to envelopes / auto-weights (tune influence volumes, cap 2 bones per vertex, auto-on-bind, auto-normalize, symmetry). No per-vertex airbrush. Deletes the weight-paint taste loop, down to a bounded pose-and-check.
- Animation: keyframed bone transforms with interpolation (slerp), authored as clips. Already the wok-anim plan.

## The pipeline

Place primitives -> compose with CSG (union/subtract/intersect) -> tri-planar UV + paint -> normal pen -> rig (skeleton + envelopes) -> animate (clips) -> sockets. Ordering rule: lock the geometry before rigging - weights bind to the bind-pose mesh, so re-running CSG or moving vertices after binding breaks them.

## Locked sub-decisions (2026-06-26)

- Modeling is CSG-CORE. Place polygon primitives (each carries a resolution / segment choice at placement, the source of vertex density), compose them with boolean union / subtract / intersect as the hero verbs. This SUPERSEDES the earlier merge=combine decision. Direct vertex manipulation is a secondary tweak tool, not the primary verb.
- The CSG kernel is a DEPENDENCY, not hand-rolled. Robust mesh booleans are a solved problem (mesh arrangements + winding numbers); a naive from-scratch boolean is the trap, and reimplementing the robust kernel is exactly the wheel the project does not reinvent. Recommendation: prototype on csgrs (pure-Rust, MIT, BSP-backed, OpenSCAD-like) behind a thin swappable boundary in wok-mesh, and escalate to Manifold (Apache-2.0, the robust C++ kernel OpenSCAD runs on, via the manifold3d / manifold-csg FFI bindings) if BSP robustness bites on real models. Both are permissively licensed (commercial-safe). Taking either is Harrison's dependency call: csgrs pulls the nalgebra / Dimforge stack (a second math lib next to the workspace's glam - convert at the boundary), Manifold adds a native C++ build. Isolating the kernel behind our own thin interface keeps it swappable.
- Materials: one material per mesh object; a multi-material item stays as several separate meshes. A prefab is therefore N meshes plus one shared skeleton; every mesh skins to that skeleton.
- Texturing is tri-planar projection on the CSG result (no unwrapper). Painting has two tools: flat per-shape colour (vertex colour) and pen-on-surface (paints into a texture addressed by the tri-planar projection).
- Normal maps are COMMITTED (the Sly-flat vs Ratchet-detail fork is resolved: we do them). Surface detail only - they never change the silhouette (that is the CSG / vertex tools). Authored by painting a height/bump map and deriving the normals; the renderer gains a tangent-space material path.
- Skinning is smooth, capped at the ENVELOPE tier (no manual per-vertex airbrush). Rigid and soft-seam are the simpler rungs; envelopes (auto-weights tuned by influence volume) are the ceiling. Runtime is linear-blend skinning regardless.
- Animation clips (running, idle, jump) are DISTINCT from prefab states (default, open, destroyed = structural shape-set variants). Both ride the prefab; separate axes. The editor authors clip data; the game owns the when-to-play state machine.
- Sockets are named attachment transforms parented to bones (hand_r, muzzle, back, head). They move with the animation for free; attaching means a child prefab's root follows the bone each frame. The editor authors the points; the game owns what attaches and when. In glTF a socket is just a node.
- Interchange is glTF: mesh, material, skeleton, weights, clips, and sockets (as nodes) all fit it. AI-generated (meshy.ai) and bought assets import through glTF. Organic, smoothly-skinned CHARACTERS are routed through import rather than the in-editor rigger, which targets props and rigid / segmented assets.

## Build sequence (verify-small bites; per-bite specs written when reached)

Phase A - Modeling (the hero):
- A1. wok-mesh: integrate the chosen CSG kernel behind a thin swappable boundary; union / subtract / intersect producing a MeshCpu. (Dependency gate - Harrison approves the crate.) Verify: cube union sphere, a subtract, an intersect; watertight result; round-trips to MeshCpu.
- A2. Editor: place primitives (with a resolution choice), compose a CSG result live, bake and render it in the viewport.
- A3. Editor: direct vertex manipulation on primitives / the result (the secondary tweak tool).

Phase B - Look:
- B1. wok-render: a material path - base-colour texture + normal map, tangent-space lighting - layered onto the banded model.
- B2. wok-mesh: tri-planar UV generation + height-to-normal derivation.
- B3. Editor: paint tools - flat colour, pen-to-texture, and the height/normal pen.

Phase C - Motion:
- C1. wok-anim + wok-render: clip data + pose evaluation + GPU linear-blend skinning (bone palette).
- C2. Editor: skeleton authoring + envelope auto-weights (soft-seam, 2-bone cap), pose-test in the editor.
- C3. Editor: the animator - keyframe clips on a timeline, saved to the prefab.
- C4. Sockets: named attachment bones authored on the skeleton.

Cross-cutting: wok-scene glTF import/export, landed once the runtime plays skinned meshes, to bring meshy / bought characters in.

## Engine implications (prerequisites; not yet built)

- wok-mesh: the CSG kernel behind a thin boundary (union / subtract / intersect to MeshCpu), tri-planar UV generation, height-to-normal derivation.
- wok-render: a material path (base-colour texture + normal map, tangent-space lighting) layered onto the banded model, and GPU linear-blend skinning (a bone-matrix palette). COMMITTED by the normal-maps and smooth-skinning decisions. Runtime skinning is one path regardless of which auth tier produced the weights.
- wok-anim: moves from deferred to planned - clip data, pose evaluation, blending.
- wok-scene: the prefab data model extends - mesh data, material refs, skeleton, skin weights, clips, sockets - layered onto the existing shapes / states / mesh-name model; glTF import and export.
- wok-physics: unaffected - picking and colliders already classify from the authored shapes.

## Deferred / open

- Which CSG kernel (csgrs prototype vs Manifold escalation) - the one open decision, pending Harrison's dependency approval. Everything else below is resolved.
- SDF representation: rejected. It would make booleans free, but it fights the (now secondary) vertex manipulation and yields rounded output; CSG on polygons is the chosen paradigm.
- Hand-rolling the boolean kernel: rejected - it is the wheel; depend on a robust kernel and build the opinionated workflow on top.
- Smooth-skinning deformation QUALITY stays a bounded pose-and-check loop; intrinsic, not removable by tooling.
- HLD integration of the engine prereqs: held until the track is build-ready (avoid documenting ahead of build).
