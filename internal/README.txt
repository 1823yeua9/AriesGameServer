Aries internal (x64 DLL, build 13.06)

Output: x64/Release/internal.dll. Loaded and supervised by the loader; no manual
injection step required. Requires a matching per-launch runtime directory, PID,
executable path, held configuration gate, and readiness event. Loading it
independently does not initialize or modify a random process.

startup.cpp ports the verified project's existing initialization sequence.
Addresses are image-base + build-specific RVA, with original-byte/table checks;
runtime object addresses are resolved anew. No per-run absolute addresses.
Client file on disk, Vanguard service, driver and tray remain unchanged.
Original dispatch tables are restored after initialization. Unsupported builds
or changed object graphs fail closed rather than silently continuing.

INSERT toggles the existing dev menu. Skins uses the loader's authenticated
loopback admin endpoint; reflection views inspect current world/controller data.
Other existing placeholder controls are not claimed as completed features.

Session / Maps also has Add bot. In a playable Range it requests one replicated
bot 500 Unreal units (5 m) ahead along the player's view yaw, at pawn height,
facing the player. It does not project onto terrain or trace through obstacles;
use it on clear, level ground. Looking up/down does not change spawn height.
The loader gives the internal and gameplay server a run-local gameplay_commands
queue. Requests expire after eight seconds and are single-flight in the menu;
the status shows the server result, not merely that a file was queued. Disk work
and waiting run off the rendering thread. Missing/stale player poses are rejected.

Viewers adds independent, OFF-by-default options:
  Cyan 3D boxes: engine-provided actor world bounds across streamed levels.
  Gold collision viewer: root/attached shape components and skeletal PhysicsAsset
    spheres, boxes and capsules transformed using live bone transforms.
    Box half extents and capsule cylinder lengths are interpreted explicitly;
    root sphere/capsule dimensions use engine-scaled getters.
  Optional entity names.

Training targets now share their live skeletal collision primitives with the
external server. The viewer and damage tests consume the same sphere, oriented
box and capsule dimensions, including local shape rotations and bone transforms.
Head/leg classification comes from AShooterCharacter's damage bone lists; other
bones are body hits. Root movement capsules and actor bounds are NOT damage shapes.
Non-query shape elements are excluded. Unsupported convex, tapered, level-set
and nonuniformly scaled skeletal shapes are reported, not approximated.

Engine getters run on the game window message thread, never Present. Round-robin
sampling: 64 actors/tick, 2 ms target (an individual getter can exceed it).
Shapes retire after a completed world sweep, not a two-second per-actor timer.
Rendering consumes immutable snapshots; engine calls do not hold the publication
lock and contention no longer skips drawing. A stopped sampler hides its entire
snapshot after two seconds. Read faults disable sampling for this run.
No viewer operation changes collision sizes, player state or server-owned values.

The collision feed stays active with the viewer closed. A separate round-robin
target pass has a 2 ms budget per 33 ms timer (one getter can exceed the budget).
It publishes at most 64 targets / 128 primitives per target via versioned local
shared memory. Both sides use zero-wait mutex acquisition. No per-frame files,
server waits, process writes, or game-thread locks spanning intersection work.
The server resolves network channels to actual actor pointers and checks world
identity, layout version, primitive validity and sample age <= 250 ms. Incomplete
or stale data rejects the shot with bot_collision_error; no old cylinder fallback.
Gold target geometry is hidden when its sample expires or is incomplete.
This uses the current skeletal pose, NOT full skeletal lag compensation. Map
occlusion, damage falloff and historical-pose rewind are separate systems.
Damage rays now use the completed reusable hitscan projectile's CreationTransform,
not CameraCachePrivate sampled after recoil. The projectile includes its actual
shot direction/spread. Seed and projectile identity are checked around the read;
incomplete updates are retried without falling back to the later render camera.
The live damage bone list includes Head AND Neck; headshot classification does
not by itself prove that a head shape should be moved.

Build depends on the existing 13.06 generated CppSDK and adapter in ../dumper7
relative to this repository. Not a standalone distributable SDK. Bundled
ImGui/MinHook retain their upstream license files.

Tests, no game or firewall writes:
  msbuild internal/tests/geometry_tests.vcxproj /p:Configuration=Release /p:Platform=x64
  internal/tests/bin/geometry_tests.exe
  msbuild internal/tests/startup_scan_tests.vcxproj /p:Configuration=Release /p:Platform=x64
  internal/tests/bin/startup_scan_tests.exe

An elevated loader run reached the main menu with both Rust services, 121/121
initialization complete, original dispatch restored, and ImGui ready. The user
confirmed Range outlines render but flicker; the snapshot/sweep correction builds
and is undergoing live validation. First sampled TrainingBot classes log actual
bone/collision dimensions once per world for the separate server hit-region audit.
The earlier standing-head calibration remains only for historical fixture tests;
the production firing path uses collision_shots and the live geometry feed.
