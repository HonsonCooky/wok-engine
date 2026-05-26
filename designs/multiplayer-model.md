# Multiplayer — Diverging Paths

This document describes the multiplayer model chosen for Unstitched and
any future games on the Wok engine that opt into this approach.

It is a game-design document, not an engine architecture document. Wok
itself has no multiplayer concept — the model is composed at the game
layer from engine capabilities that exist for other reasons (single
player, save/load, cutscenes, AI). The final section enumerates which
capabilities the model uses, all of which Wok already provides.

---

## The Model

All clients start from a shared state. Each client runs its own local
simulation, and from the starting state each one takes its own path. The
paths diverge freely. Sometimes two paths touch briefly and influence
each other. At certain moments — designed by the game — all paths merge
to agree on a shared fact. At the end of the session, the paths merge
fully into a final outcome.

There is no canonical world state being maintained on a server. There
are N parallel simulations, loosely linked. Each client's local
simulation is the truth for that client.

The network is thin. Clients exchange two things across their paths:

- **Intents** — what a player is trying to do. Movement direction,
  button presses, intended targets. Received as suggestions by other
  clients, interpreted by their local AI in their local world state.
- **Events** — explicit things that have happened. A conversation, a
  kill, an objective completed, a scene transition. Some events are
  locally authoritative; some require merge.

---

## The Actor Model

Every character in every client's world — local player, remote player,
enemy, NPC — is the same kind of object: an actor. Actors have position,
velocity, physics integration, animation, and an input source. The
source is what varies:

- Local player actors receive input from the local controller.
- AI actors (enemies, NPCs) receive input from local behavior logic.
- Remote player actors receive intent suggestions from the network,
  interpreted by local AI in the local world state.

The engine does not distinguish these cases. There is no "bot" type and
no "remote player" type. There is one actor system. The game layer
plugs in input sources.

The consequence: remote inputs propose, local AI disposes. If a remote
client suggests "moving forward, holding fire button," the local AI on
the receiving machine decides what actually happens — whether the shot
lines up, whether there's a wall, whether the target is where the
suggester thinks. This is what makes the model robust against
suggestion-based attacks and tolerant of latency.

---

## Merging Paths

Most of the time paths diverge and no reconciliation is needed. The
moments where paths must agree are designed by the game, and the engine
supports three patterns the game can pick from per outcome:

**Divergent-local.** Both local truths stand. Two players in a shootout
each locally win their fight — neither is reconciled, both go home
feeling like the winner. Default for combat outcomes in cooperative or
social-competitive play.

**Server-arbitrated.** The server picks one outcome; all clients accept
the verdict. Required when downstream logic depends on a singular truth:
ranked matches, finite resources, leaderboard exactness.

**Event-merged.** Both events are accepted and combined. Both players
get credit for the kill, both contribute to a shared counter. Useful
for cooperative scoring.

The game designer picks the pattern per event type. The engine supports
all three; the game wires its events accordingly.

Some merges are non-negotiable — level transitions, quest completions,
scripted story moments, persistent progression. These are **anchor
events**: explicit synchronization points broadcast by an authoritative
source, accepted by all clients without local override. Anchors are what
keep the divergent paths telling the same story.

---

## Off-Screen Freedom

Divergence between clients only matters when it's visible. When two
players cannot see each other — different rooms, occluded terrain,
different scenes — their respective actors can be at completely
different positions in each other's worlds, doing different things, and
no one notices. Divergence accumulates freely.

When lines of sight converge, the model resyncs the relevant state:
remote actors snap to plausible locations, current actions align. Brief
settling reads as "they were doing something off-camera," not as a
network artifact.

The engine exposes a mutual-visibility query: given two actor IDs, are
they currently in each other's view frustums or close enough that
divergence would be noticed? The multiplayer layer uses this to trigger
resyncs.

---

## Spectating

A spectator is a client whose path doesn't emit — they receive
suggestions and events from other clients, run their local simulation,
and observe. They don't send intents. They don't add an actor to other
clients' worlds.

Spectators choose what to receive. Follow one player and see the match
through their perspective. Follow several and see a composite. Follow
only the server's validated event stream and see a consensus
reconstruction. None of these is canonical; each is internally
consistent.

This scales the same way playing does — each viewer runs their own
local sim. No central broadcast infrastructure required. Closer to
Twitch (pick a streamer, watch through their perspective) than to
television (one production truck, one feed).

---

## Security

The architecture changes which attacks matter. Most multiplayer cheating
works by manipulating shared state, and there is no shared state.

**Structurally impossible:** item duplication that propagates, position
manipulation visible to others, score injection in shared rankings,
race conditions on consensus, wallhacks visible to others, aimbot
effects guaranteed as hits, speed hacks affecting other clients.

**Defensive principles:**

1. **Local-only cheating is not a security concern.** If a cheat only
   affects the cheater's own client, we don't engineer to prevent it.
2. **Suggestions are interpreted, never executed.** Network inputs are
   suggestions to local AI, which decides what happens in the local
   world. Implausible suggestions get re-interpreted according to local
   truth.
3. **Server validates plausibility, not simulation.** The server checks
   whether received events are plausible given recent context. Cheap on
   commodity hardware, catches the gross majority of attacks.
4. **Anchor events are server-validated.** Things that must agree
   across clients (level transitions, persistent progression) are
   validated before propagation. This is where the validation budget
   gets spent.
5. **Rate limiting and anomaly detection are standard.** Generic
   network-security practices apply.

**Residual concerns:** social griefing (handled by friends-only / kick
mechanisms, not security) and suggestion-DOS (handled by rate limiting).

The security cost shifts to server-side plausibility validation, which
is cheap.

---

## Engine Capabilities Required

The model imposes no engine requirements beyond what Wok already needs
for single-player. Multiplayer is composed by the game from capabilities
that exist for other reasons:

- **Actor system with pluggable input sources** — Wok already provides
  this for local players vs AI characters. Multiplayer just adds
  "interpreted network suggestions" as another input source.
- **Deterministic local simulation** — Wok already needs this for AI
  replay validation and save/load. Multiplayer benefits for free.
- **Snapshottable world state** — Wok already needs this for save/load.
  Multiplayer uses the same mechanism for join-in-progress and
  anchor-event resync.
- **Mutual-visibility queries** — Wok already needs these for frustum
  culling and AI line-of-sight. Multiplayer uses them to gate resync.

There is no multiplayer subsystem in Wok. The game implements
multiplayer by wiring these single-player capabilities together with
its own network layer. Wok never sees the network, never knows a
suggestion came from elsewhere, never distinguishes a remote-input
actor from a local-AI actor.

This is the same pattern as cutscenes, save/load, and triggers: Wok
provides primitives; the game composes them.