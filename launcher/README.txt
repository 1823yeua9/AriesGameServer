Aries launcher

760x450 borderless front end inspired by SPT's legacy layout: slim title
bar, equal-width profile/session cards, centered bottom launch bar. All text
is centered within its region, including individual wrapped lines. Custom-drawn
buttons retain keyboard navigation; minimize/close use line icons, not glyphs.
The backdrop retains
KDE Dual Kawase blur, strength 14, grain disabled. No SPT assets/source bundled.
Reference: https://github.com/sp-tarkov/launcher/tree/3.11.x-dev/project/SPT.Launcher

Build the C++ solution in Visual Studio with Release/x64, or in its Developer shell:
  msbuild AriesGameServer.slnx /p:Configuration=Release /p:Platform=x64
Build both Rust services from the repository root:
  cargo build --release --manifest-path server/Cargo.toml
  cargo build --release --manifest-path server/platform/Cargo.toml
Output: x64/Release/launcher.exe (next to loader.exe and internal.dll)
The full solution now includes launcher, loader and internal.

Start game delegates to x64/Release/loader.exe; the UI does not inject modules,
start servers independently or control Vanguard. Check setup invokes read-only
preflight. Stop session closes only the owned launch tree. Existing VALORANT
instances are refused, never killed by name.

Settings selects a .val/.json profile, opens it in Notepad, or creates aries.val
without overwriting an existing file. The explicit Administrator firewall button
requests elevation only for the bounded --install-firewall mode, never automatic
game startup. Closing during that short setup does not cancel its elevated
process. Logs shows this launcher's latest 16 KB of loader output at 4 Hz.

Firewall setup also registers "Aries Offline Firewall Reset", a SYSTEM boot task
that disables only rules in the Aries offline containment group with Aries rule
names. It runs Windows PowerShell with a fixed inline command, not a script or
binary in this writable project. Windows startup disables the rules even if the
launcher is never opened; it does not turn off Windows Firewall. After reboot,
use Set up offline firewall again before launching an isolated session. Ordinary
launch/check only verifies rules and the reset task; it never re-enables rules.

Local username is editable on the home card. Save username persists it to the
selected profile; Start game also saves it before starting the loader. Changes
apply on the next launch. Names support 1-32 Unicode characters, excluding #,
control characters and surrounding whitespace. The LOCAL tag and account UUID
stay unchanged, preserving inventory and loadout. Old profiles use OfflinePlayer.
The platform uses the name consistently in lobby lookups, chat/presence and
local identity tokens. This does not rename a production Riot account.

Default paths are relative to the repository root. Platform and gameplay receive
the same absolute catalog/state paths, preserving the existing
local_backend/offline_skin_state.json inventory. Internal receives the actual
per-run admin endpoint/token instead of a hardcoded port or shared credential.

Backdrop: GPU-local DXGI desktop duplication, excluding this window to prevent
recursive blur. Ordinary screenshots omit the launcher. No DWM injection or
undocumented compositor hooks. No desktop image files during normal operation.
Windows 10 2004+; primary monitor only, rotation handled. SDR BGRA8; HDR and
cross-monitor parity are not claimed. See reference/NOTICE.txt for KDE licensing.

Verification modes (never launch a game):
  --self-test   Hardware/WARP vs CPU reference of pinned GLSL. Constant, edge,
                gradient, color adjustment and grain fixtures; max 3/255,
                mean 1/255 RGB tolerance.
  --smoke-test  Own 400x400 color-changing/moving backdrop. Checks all 160000
                pixels for feedback or stale-coordinate failures.
  --preview     Normal UI rendered to its own framebuffer, saved as
                blur-artifacts/launcher-preview.bmp, then exits.

All test modes return nonzero on failure. Normal status: blur-artifacts/launcher.log.
Startup errors: launcher-error.txt. Check timestamps, not stale artifacts.

Release builds, numerical/scanner/loader tests and rendered preview pass.
An elevated live loader run reached MainMenuV2 with both Rust services and the
internal menu. The revised centered layout was built and checked via --preview.
Range viewer/damage acceptance is separate from launcher rendering tests.
