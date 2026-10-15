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

Viewers adds independent, OFF-by-default options:
  Cyan 3D boxes: engine-provided actor world bounds across streamed levels.
  Gold collision viewer: root/attached shape components and skeletal PhysicsAsset
    spheres, boxes and capsules transformed using live bone transforms.
    Box half extents and capsule cylinder lengths are interpreted explicitly;
    root sphere/capsule dimensions use engine-scaled getters.
  Optional entity names.

Client collision shapes are NOT claimed to match the external server's fixed
head/body/leg hit approximations. Those have not been resized without measured
evidence. Unsupported convex, tapered, level-set and nonuniformly scaled skeletal
shapes are counted, not guessed. Component traversal is cycle-checked, 64 max.

Engine getters run on the game window message thread, never Present. Round-robin
sampling: 64 actors/tick, 2 ms target (an individual getter can exceed it).
Shapes retire after a completed world sweep, not a two-second per-actor timer.
Rendering consumes immutable snapshots; engine calls do not hold the publication
lock and contention no longer skips drawing. A stopped sampler hides its entire
snapshot after two seconds. Read faults disable sampling until re-enabled.
No viewer operation changes collision sizes, player state or server-owned values.

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
The separate gameplay server now uses the measured sphere/capsules for standing
TrainingBot and dummy heads instead of the oversized legacy head sphere. Torso
and legs remain approximations; this is not animated skeletal hit authority.
Chest/head regression tests pass. The updated live server recorded body hits at
40 and head hits at 160 with the Vandal; the user accepted the visible damage
and viewer behavior as working well enough on 2026-10-14.
