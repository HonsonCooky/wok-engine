# Multiplayer - Diverging Paths

Multiplayer model for games on the Wok engine that opt into this
approach. Game-design document; not engine architecture. Wok itself
has no multiplayer concept - the model is composed at the game
layer from engine primitives that exist for other reasons.

---

## The Model

All clients start from a shared state. Each runs its own local
simulation; each takes its own path from the starting state. Paths
diverge freely. At designed moments, paths merge to agree on shared
facts. At session end, paths merge into a final outcome.

No canonical world state is maintained on a server. N parallel
simulations, loosely linked. Each client's local simulation is that
client's truth.

The network is thin. Clients exchange:

- **Intents** - what a player is trying to do (movement direction,
  button presses, intended targets). Received as suggestions,
  interpreted by the receiver's local AI in its local world state.
- **Events** - explicit things that have happened (a conversation,
  a kill, an objective completed, a scene transition). Some events
  are locally authoritative; some require merge.

---

## The Actor Model

Every character in every client's world - local player, remote
player, enemy, NPC - is the same kind of object: an actor. Actors
have position, velocity, physics integration, animation, and an
input source. The source varies:

- Local player actors: input from the local controller.
- AI actors (enemies, NPCs): input from local behavior logic.
- Remote player actors: intent suggestions from the network,
  interpreted by local AI in the local world state.

The game's actor system does not distinguish these cases. One actor
system, input sources plugged in per actor. The engine has no actor
concept at all - actors live in the game's pool; the engine provides
only the math primitives the game's integration loop calls.

Consequence: **remote inputs propose, local AI disposes.** If a
remote client suggests "moving forward, holding fire," the local AI
on the receiving machine decides what actually happens - whether the
shot lines up, whether there's a wall, whether the target is where
the suggester thinks. Robust against suggestion-based attacks;
tolerant of latency.

---

## Merging Paths

Most of the time, paths diverge and no reconciliation is needed.
When paths must agree, the game picks one of three patterns per
outcome:

- **Divergent-local.** Both local truths stand. Default for combat
  outcomes in cooperative or social-competitive play.
- **Server-arbitrated.** Server picks one outcome; all clients
  accept. Required when downstream logic depends on a singular
  truth (ranked matches, finite resources, leaderboards).
- **Event-merged.** Both events accepted and combined. Useful for
  cooperative scoring.

The engine supports all three; the game wires its events
accordingly.

Some merges are non-negotiable - level transitions, quest
completions, scripted story moments, persistent progression. These
are **anchor events**: explicit sync points broadcast by an
authoritative source, accepted by all clients without local
override.

---

## Off-Screen Freedom

Divergence between clients only matters when visible. When two
players cannot see each other, their actors can be at completely
different positions in each other's worlds and no one notices.

When lines of sight converge, the model resyncs relevant state:
remote actors snap to plausible locations, current actions align.

Mutual visibility is a game-side query composed from engine math
primitives (frustum tests, distance queries). Given two actors in
the local pool: are they currently in each other's view frustums or
close enough that divergence would be noticed? Answer gates resync.

---

## Spectating

A spectator is a client whose path doesn't emit. They receive
suggestions and events from other clients, run their local
simulation, and observe. They don't send intents. They don't add an
actor to other clients' worlds.

Spectators choose what to receive: one player's perspective,
multiple for a composite, or only the server's validated event
stream for a consensus reconstruction. None is canonical; each is
internally consistent. Scales the same way playing does - each
viewer runs their own local sim. No central broadcast infrastructure
required.

---

## Security

Most multiplayer cheating manipulates shared state; there is no
shared state.

**Structurally impossible:** item duplication that propagates,
position manipulation visible to others, score injection in shared
rankings, race conditions on consensus, wallhacks visible to others,
aimbot effects guaranteed as hits, speed hacks affecting other
clients.

**Defensive principles:**

1. Local-only cheating is not a security concern.
2. Suggestions are interpreted, never executed. Implausible
   suggestions get re-interpreted according to local truth.
3. Server validates plausibility, not simulation. Cheap on commodity
   hardware; catches the gross majority of attacks.
4. Anchor events are server-validated.
5. Rate limiting and anomaly detection are standard.

**Residual concerns:** social griefing (friends-only / kick
mechanisms) and suggestion-DOS (rate limiting).

---

## Engine Primitives Required

The model imposes no engine requirements beyond what Wok already
provides for single-player. Multiplayer composes from primitives
that exist for other reasons:

- **Actor systems with pluggable input sources** - game-side. The
  engine provides math primitives the game's integration uses
  (collision, gravity, integration math); the game's actor system
  plugs input sources in per actor. Multiplayer adds "interpreted
  network suggestions" as another input source.
- **Deterministic primitives** - HLD principle #6. Game's
  fixed-timestep simulation loop calling these produces
  deterministic gameplay across clients.
- **Per-crate state accessors for world capture** - engine exposes
  accessors on each crate holding state (chunk membership from
  wok-content, dynamic light pool from wok-light, voice pool from
  wok-audio, etc.). Game composes these with its own state into
  whatever capture format it needs. Join-in-progress and
  anchor-event resync use the same composition as save/load.
- **Spatial query primitives** - frustum math, raycasts, AABB
  queries. Game composes for mutual-visibility queries that gate
  resync.

Wok has no multiplayer subsystem. The game implements multiplayer
by composing single-player primitives with its own network layer.
Wok never sees the network and has no actor concept at all -
remote-input actors and local-AI actors are just entries in the
game's actor pool with different input source plugs.

Same pattern as cutscenes, save/load, and triggers: Wok provides
primitives; the game composes.
