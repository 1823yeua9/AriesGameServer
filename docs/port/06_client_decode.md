# 06 — Decoding what the client sends

Spec for the Rust decoder of client-to-server traffic, ported from the Python Range responder. Everything below describes **what the Python does today on the production flag set**. Python source paths are relative to `local_backend/`. All line numbers were read on 2026-10-14.

Conventions:
- **Bit order.** Every bit read is LSB-first: bit `n` of a buffer is `(buf[n/8] >> (n%8)) & 1`, and a multi-bit field takes its low bit first (`range_client_input.py:14-22`, `range_udp_handshake.py:275-282`).
- **Decoded packet.** This is the plaintext after AES-GCM and the inverse Ares transform. Its bit count includes the UE terminator bit. The high bits of its last byte are **not** zero (see §2.4).
- **`source`** is the client's `(ip, port)`. Every per-connection dict in Python is keyed by it.
- All vectors in §13 were produced offline by importing the Python modules (no live process). The script is `scratchpad/vectors.py` from this session, and the vectors are pasted below.

---

## 0. What the client sends and what Python actually uses

On the production path, these are the **only** inputs derived from client packets:

| Input | Where it comes from | Consumer |
|---|---|---|
| Handshake initial and response (connectionless) | `parse_handshake` (`range_udp_handshake.py:285-318`) | challenge and ack (transport doc; not repeated here) |
| Client packet sequence, stateless header, ack history | outer decode plus packet header (§2, §3) | every server send (`acked_client_sequence`, `ack_history_mask`, `stateless_header`) |
| Client's acked server sequence (bits 4..17 of the header) | **only** the 77-bit packet that triggers the NMT_Challenge (§9) | seeds `next_server_sequences` and `initial_actor_channel_sequences` |
| First bunch's `control_message_type`, plus payload bit counts 232 and 77 | §4.2 | Hello capture, Challenge, Welcome, Join ack (§9) |
| Input-event streams in the first bunch when it is on ch8 | §8 | ClientAck input RPC (input_ack_cursor_trial) |
| ServerPurchase (ch5 field 156), in any bunch of the packet | §6 | gun and item loader |
| ServerUpdateShootingRangeSettings (ch5 field 176), in any bunch | §7 | GameState infinite-ammo bool |

Things that are **not** decoded from packets: movement (ServerMove), predicted equippable states, ammo, firing, camera, ability use. Python reads all of them from client memory with ReadProcessMemory (RPM): `read_saved_moves`, `EquippableStateReader`, `read_input_replication` and others. The move ack (`range_udp_handshake.py:4412-4535`) takes its move number from `read_saved_moves`, not from the ch8 packet. Python also ignores the client's acks of server packets, apart from the one 77-bit packet above. It never retransmits.

### 0.1 Production vs dead (decode-related)

Production (set by `launch_range.py:29-53, 215-219`): `--raw-challenge --raw-envelope-bytes 4 --raw-terminal-only --live-client-pid --live-client-image-base --session-ack-probe --store-purchases --range-settings --movement-packet-capture --input-ack-cursor-trial` (plus the setup flags owned by other docs). `session_ack_probe` enables the challenge, welcome and join-ack branches (`range_udp_handshake.py:2111, 2115, 2143, 2158`).

Dead, one line each:
- `derived_key_gcm`: GCM-wrapped challenge (`:4549-4557`). Off.
- `ack_probe_once` / `challenge_probe_once` / `welcome_probe_once` / `join_ack_probe_once`: superseded by `session_ack_probe`. `ack_probe_once` would send `build_server_ack_probe` instead of the challenge (`:2131-2133`). Off.
- `VALORANT_RANGE_CAPTURE_LOGIN=1`: logs the decoded NMT_Login hex once (`:2079-2085`). Logging only. Off.
- `VALORANT_RANGE_HEARTBEAT_HZ` (default 0): heartbeats built from `latest_authenticated_packets` (`:1366-1369, 1416-1437`). Off.
- `VALORANT_RANGE_QUIET_PACKETS`: a console filter only (`:1365, 1373-1400`). Not set by the launcher.
- `move_ack_value_source="timestamp"`, `move_ack_fixed`, `follow_client_move(_once)`, `track_client_location`: RPM consumers, not packet decode. Off.
- Fields 157 (ServerRemove) and 161 (ServerSell): decoded and logged, never acted on (`:1871`).
- Settings fields 172, 173, 174, 175 and 177: decoded and logged, never acted on (`:1940`).
- `attach_match_state.py`, `inspect_captured_bunches.py`: offline tools that reuse these decoders. Not part of the server.
- `launch_range.py --pawn-authority-role`: replaces the reconcile flag set, so `--input-ack-cursor-trial` (and the move ack) are not passed (`launch_range.py:206-209`). Off.
- `VALORANT_RANGE_DROP_FLAGS` (launcher A/B switch that deletes responder flags, `launch_range.py:212-213`): unset in production. If it ever removes `--input-ack-cursor-trial`, `--store-purchases` or `--range-settings`, the matching decoder below is gated off.
- `RANGE_ACR_ACTOR_INSPECTION_PROBE`, `RANGE_EQUIP_STATE_TRIAL`, `RANGE_BASE_PISTOL_SKIN_TRIAL`: env-gated setup experiments (`:736-740, 865`). They read no client packet bytes. Off.

---

## 1. Per-datagram pipeline (exact order)

Source: `run_probe`, `range_udp_handshake.py:1440-4579`. The responder is receive-driven: `recvfrom(2048)` with a 0.25 s timeout (`:1412-1413, 1442`).

1. Drop the datagram unless `source[0] == "127.0.0.1"` (`:1460`). `ConnectionResetError` is ignored (`:1455-1459`).
2. **Key acquisition** (once per process, not per connection). While `client_send_key is None`, scan the client process for the Ares packet-handler vtable `image_base + 0xA5F3BB0` (`:1472`). Then `client_send_key = handler[0x28..0x48]` and `receive_key = handler[0x48..0x68]` (`check_live_packet_keys.py:74-75`). On failure (`OSError`, `RuntimeError`, `ValueError`) the event is logged as `handler_state: not_ready` and the datagram is dropped (`:1476-1481`). This runs **before** `parse_handshake`, so the connectionless handshake datagrams are dropped too until the keys are found. The scan is synchronous and bounded at 30 s / 4 GiB per attempt (`check_live_packet_keys.py:42-52`); Rust must not run it on the tick thread. On success Python prints `read-only Ares handler=0x… receive-key fingerprint=<sha256(receive_key) hex[:12]>` (`:1482`). The keys are never re-read, so a client reconnect with a new handler leaves every later packet unauthenticated. No per-connection state is reset by a new handshake from the same `source` either (`pending[source]` is overwritten, nothing else).
3. Try `parse_handshake(payload)` **first** (`:1484`). If it succeeds, take the connectionless handshake path (`:4538-4572`). Session packets must fail it with `ValueError`. It raises when the payload is under 57 bytes or its first 8 bytes are not zero (`:286-287`), or on the later bit and version checks (`:298-314`). The content of a session packet's 64-bit prefix is not recorded (open question 1).
4. On that `ValueError`: `event.update(inspect_client_ares_packet(payload, client_send_key))` (`:1487`; §2-§4).
5. If authenticated:
   - `ack_history_mask = update_received_packet_history(recent_client_sequences[source], client_packet_sequence)` (`:1490-1492`; §3.2).
   - `latest_authenticated_packets[source] = (seq, stateless_header, ack_history_mask, monotonic())` (`:1493-1495`). It is consumed only by the heartbeat, which is off.
   - **Input ack** (§8.5) (`:1499-1563`).
   - **Authority block**, gated by `receive_key known && live pid && image base && source in client_restart_sent_sources` (`:1566-1568`). It contains, in this order: equippable mirror, input correction, the off server-performance RPC, range HUD, streaming, ult points, **purchases** (§6.5, `:1851-1907`), **settings** (§7.4, `:1913-1946`), command queue, HUD refresh, projectiles, bots and abilities. Only purchases and settings read packet bytes.
6. `VALORANT_RANGE_CAPTURE_LOGIN` (dead) (`:2079-2085`).
7. **Movement and input capture logging** (`:2086-2106`; §8.6).
8. The **control `elif` chain** (`:2107-2184`; §9): not authenticated, then Hello capture, Challenge, Welcome, Join ack, session ack plus the setup stages.
9. `events.append`; if the handler took ≥ 25 ms, add `handler_elapsed_ms` (rounded to 3 places); `emit_event` (prints the JSON event); `send_due_server_heartbeats()` (`:4573-4579`; heartbeat off).

Every server send inside steps 5-8 carries `acked_client_sequence = this datagram's client_packet_sequence`. **The ack is "the latest datagram received", not "the highest sequence".** An out-of-order arrival makes the acked sequence go backwards. Python does this, so Rust must reproduce it for bit equality. The history mask and the server sequence differ by sender:

| Sender | Server sequence | History word sent |
|---|---|---|
| Input ack, authority-block `send_reliable`, session ack, setup stages, move ack | `next_server_sequences[source]`, then `+1 & 0x3FFF` | `ack_history_mask` from step 5 |
| Challenge (§9) | `(((H_client >> 4) & 0x3FFF) + 1) & 0x3FFF` from **this** 77-bit packet (`:656`); `next_server_sequences` is then **set** to that `+1 & 0x3FFF` | constant `3` (`build_server_control_probe`, `:662`) |
| Welcome (§9) | `next_server_sequences[source]` | constant `7` (`:662`) |
| Join ack (§9) | `next_server_sequences[source]` | constant `3` (default of `build_server_session_ack`, `:613`) |

> 128 Hz: the Rust server ticks at 128 tps. To keep Python's semantics, drain every datagram that arrived since the last tick and run each one through this pipeline **in arrival order**, one at a time. All per-connection state (ack history, latest sequence, dedup sets) advances per datagram, not per tick. The wall-clock gates in this doc (0.05 s, 0.1 s, 0.2 s, 0.25 s, 3.0 s) are measured on a monotonic clock; Python reads the clock per datagram (`now` at `:1569`, `time.monotonic()` in the control chain), so read it per datagram, not once per tick. Nothing in this doc sends on a tick without a datagram (the heartbeat is off): an idle tick sends nothing.
>
> Python blocks inside some handlers. The key scan is one. `inspect_packet_notify` before the Challenge took 4797 ms in `responder_24252`, and the next datagrams were all handled at elapsed 65.907. Rust should not reproduce those stalls. They do not change the bits of the frame being built. They do change what the client sends next: Hello and ack retransmits, and the extra history words of §3.1. So timing-dependent traffic is not expected to match a Python run packet for packet.

---

## 2. Layer 1: outer datagram (`decode_client_ares_packet`, `range_udp_handshake.py:445-473`)

### 2.1 Layout (client to server)

```
bit 0          64 bits   prefix ("reliability_bit_count"); ignored, never validated
bit 64          6 bits   stateless header -> returned as `stateless_header`
bit 70         32 bits   GCM counter (u32, LE bit order)
bit 102        32 bits   GCM tag, first 4 bytes of the 16-byte tag (LE bytes)
bit 134    ceil(P/8)*8   ciphertext bytes (P = plaintext bit count)
...                      remaining trailer bits (0xCC pattern, see 2.4)
last set bit             outer terminator
```
- `packet_bit_count = (len-1)*8 + bit_length(payload[-1]) - 1`. This is the index of the highest set bit, i.e. the outer terminator (`:452`).
- `P = packet_bit_count - 64 - 6 - 72` (72 = counter 32 + tag 32 + trailer 8) (`:453-456`).
- Reject (`None`) when: `len(payload) < 19`, `len(key) != 32`, `payload[-1] == 0`, `P <= 0`, or `ceil(P/8) > 2048` (`:450, 458`).
- **`P > 8192` with a valid tag is not rejected**: GCM succeeds, then `inverse_short_packet` raises `ValueError("unsupported packet bit count")` (`range_udp_transform.py:126-127`). The call at `:1487` sits inside the `except ValueError` of `parse_handshake`, so nothing catches it and the responder dies. Verified offline: a validly tagged 1043-byte datagram with P = 8200 raises. `recvfrom(2048)` allows P up to 16242. Real client packets stay under ~1000 bytes (P < 8000), so this never happened in the logs. Rust: treat `P > 8192` as invalid (drop, log `invalid_or_unverified`); this diverges only where Python would crash.
- The ciphertext is read as `ceil(P/8)` whole bytes starting at bit 134 (`:463`). When `P % 8 != 0`, the last ciphertext byte spans into the trailer bits.
- The server-to-client direction has a **32-bit** zero prefix (`range_server_bunch.py:1610`). The client-to-server direction has **64**. Do not share the framing code blindly.

### 2.2 AES-GCM

- AES-256, key = `client_send_key` (32 bytes, from client memory; §1 step 2).
- Nonce is **8 bytes**: `struct.pack("<II", counter, 0x31ED27AB)` (`:464`). An 8-byte IV means GCM's GHASH-derived J0 path (not the 96-bit fast path).
- The tag is truncated to 4 bytes (`min_tag_length=4`): the first 4 bytes of the computed tag are compared. There is no AAD.
- An `InvalidTag` returns `None`, and the event becomes `{"ares_inspection": "invalid_or_unverified"}`.
- Rust note: the `aes-gcm` crate does not accept 4-byte tags. Build GCM from `aes` + `ghash` + `ctr`:
  - `H = E_K(0)`.
  - `J0 = GHASH_H(IV‖0^64 ‖ 0^64‖be64(64))`.
  - Keystream from `inc32(J0)`.
  - `T = E_K(J0) ^ GHASH_H(C‖pad ‖ be64(0)‖be64(8·|C|))`.
  - Compare `T[0..4]`.
  
  The vectors in §13.2 pin this.
- The other direction (server to client) uses nonce dword `0x7908CBDD` and `receive_key` (`:420-421`). That is not part of this doc.

### 2.3 Inverse Ares transform (`inverse_short_packet`, `range_udp_transform.py:124-171`)

`decoded = inverse_short_packet(plaintext, P, counter)` (`:472`). The counter is the GCM counter read from the wire; server packets use 0.

```
M32 = 2^32-1, M64 = 2^64-1, MUL = 0x2545F4914F6CDD1D
initial_state(c):                       # :52-61
  c &= M32
  bf = (c + 38722752) & M32
  mf = bf ^ (((bf ^ (bf >> 15)) >> 12) & M32)
  mf ^= ((c - 64) << 25) & M32          # (c-64) wraps mod 2^32 when c < 64
  ms = c ^ ((c << 25) & M32)
  ms ^= ((c ^ (c >> 15)) >> 12) & M32
  return (MUL*mf & M64, MUL*ms & M64, selector=c)
advance_state(f, s):                    # :64-69
  combined = (f + s) & M64; mixed = f ^ s
  f' = (mixed ^ (mixed << 14) ^ rotr64(f, 9)) & M64
  s' = rotr64(mixed, 28)
  selector = (combined >> 32) & M32
inverse(P bits):
  for each of P/64 u64 words (LE):   v = S⁻¹w(v); v = (rev_wide(v) + rotr32(sel,5)) & M64
                                     v = S⁻¹w(v); v = (v - rotr32(sel,3)) & M64
                                     v = S⁻¹w(v); v = (v - rotr32(sel,1)) & M64; advance
  then at most one u32 (rem>=32):    v = S⁻¹d(v); v = (rev32(v) + rotl32(sel,5)) & M32
                                     v = S⁻¹d(v); v = (v - rotl32(sel,3)) & M32
                                     v = S⁻¹d(v); v = (v - rotl32(sel,1)) & M32; advance
  then each whole byte:              v = S⁻¹b[b]; v = rev8(v); v = (v + 27*sel) & 0xFF
                                     v = S⁻¹b[v]; v = (v - 51*sel) & 0xFF
                                     v = S⁻¹b[v]; b = (v - 11*sel) & 0xFF; advance
  final partial byte (r = P%8 > 0):  b ^= ((1<<r)-1) & (sel ^ 0xC0)
```
- The rotations are **right** for 64-bit words and **left** for 32-bit words. `rotr32` and `rotl32` act on the 32-bit selector, and the result is zero-extended before the 64-bit add.
- `S⁻¹w` = `wide_sbox_inverse`, `S⁻¹d` = `word_sbox_inverse`, `S⁻¹b` = `byte_sbox_inverse`. Copy them verbatim from `range_udp_transform.py:13-18`. Each is applied per byte, little-endian (`substitute_bytes`, `:46-49`).
- `rev_wide(v)` (`:37-43`):
  ```
  a = (v>>1) ^ (((v>>1) ^ (2v)) & 0xAAAAAAAAAAAAAAAA)
  b = (a>>2) ^ (((a>>2) ^ (4a)) & 0xCCCCCCCCCCCCCCCC)
  c = (b>>4) ^ (((b>>4) ^ (16b)) & 0xF0F0F0F0F0F0F0F0)
  d = (c>>8) ^ (((c>>8) ^ (c<<8)) & 0xFF00FF00FF00FF00)
  rev_wide = rotr64(d & M64, 32)
  ```
  This is **not** a full 64-bit reverse.
- `rev32` and `rev8` are plain bit reversals (`reverse_bits`, `:33-34`).
- The transform does not touch bits above `P` in the final byte.

### 2.4 Trailer bits in the last byte

The client sets the high `8 - P%8` bits of the last plaintext byte to the low bits of `0xCC` before encrypting. The inverse transform only XORs the low `P%8` bits, so **`decoded[-1]` keeps those trailer bits**. An example is the fixture's last byte `0x64`: low 3 bits `100`, high 5 bits `0b01100 = 0xCC & 0x1F`. Every reader is bounded by `P`, so this is harmless. Keep the bytes as-is so the fixture hex matches byte for byte.

### 2.5 Return value

`(decoded: bytes[ceil(P/8)], P, counter, stateless_header = bits 64..69 of the datagram)`.

---

## 3. Layer 2: packet header (`inspect_client_ares_packet`, `range_udp_handshake.py:541-557`)

### 3.1 Fields

| Decoded bits | Width | Meaning | Python |
|---|---|---|---|
| 0..31 | 32 | header word `H` | `client_packet_sequence = (H >> 18) & 0x3FFF` (`:547`) |
| | | `(H >> 4) & 0x3FFF` = client's acked server sequence | read only for the Challenge (§9) |
| | | `H & 0xF` = history word count minus 1 | **never read**; Python assumes 0 |
| 32..63 | 32 | ack history bitmap | never read |
| 64 | 1 | has packet info | §4.1 |
| 65..74 | 10 | packet info (jitter) | skipped |
| 75 | 1 | flag | skipped (always consumed) |
| 76.. | | first bunch | §4 |

Python parses at **fixed offsets**: it never reads `H & 0xF`, and it reads bit 75 even when bit 64 is 0. Rust must do the same for parity. The logs show both assumptions failing:

- The 16923 unique full captures (`decoded_packet_hex`, gameplay ch5/ch8 traffic; §13) all have `H & 0xF == 0`, history `0xFFFFFFFF`, bit 64 = 1 and bit 75 = 1.
- Across **all** 4,391,360 unique `control_prefix_hex` values in `runtime/*.out|*.log` (first 12 decoded bytes of every authenticated packet):
  - `H & 0xF` is 1, 2 or 3 in **1399** of them (1112 / 255 / 32). Every one has the extra history words `0xFFFFFFFF`, e.g. `0109d5daffffffffffffffff`. Python reads that history word as bits 64..95: bit 64 = 1, 10 bits, bit 75, then bControl = bOpen = bClose = 1 at 76-78. It logs `{"bunch_start_bit": 76, "bunch_is_close": true}`, and `iter_bunches` returns `[]`. **Nothing in such a packet is decoded.** No input ack, purchase or settings RPC, yet the packet is acked, so any reliable RPC in it is lost. They come in bursts when the client has gone > 32 packets without an ack (for example during Python's multi-second handler stalls, `responder_24252` @392.875).
  - Bit 64 = 0 (no packet info) with `H & 0xF == 0` in **281** of them, e.g. `c079ddb7ffffff01a840c96d` (P = 1567, `responder_24252` @76.204, history `0x01FFFFFF`). Python's unconditional bit-75 read puts the bunch at 66 and logs garbage (channel 4738, partial, `bunch_end_bit` 3381 > P). Reading the flag only when bit 64 = 1 (bunch at 65) gives ch5, reliable, partial, ChSequence 3529, which is plausible. That is evidence, not proof, that the real reader skips bit 75 when bit 64 = 0 (open question 2).
- Handshake-phase packets have history words other than `0xFFFFFFFF` (`0`, `3`, `7`, `0x1F`, … in `responder_24252` @60.9-75). The history word is never read, so this has no effect.

`inspect_client_ares_packet` returns:

```
{"ares_inspection": "authenticated", "ciphertext_length": len(decoded), "counter": counter,
 "client_packet_sequence": seq, "stateless_header": sh, "payload_bits": P,
 "control_prefix_hex": decoded[:12].hex()}
```

If `P >= 128`, it also merges `inspect_control_bunch(decoded, P)`, or `{"bunch_parse_error": str(e)}` on `ValueError`. On a parse error **no** bunch key is merged, so `channel_index` and `control_message_type` are absent and every gate below that reads them is false. With `P < 32` the `struct.unpack_from("<I")` raises `struct.error`, which nothing catches (the responder dies). The client never sends fewer than 77 bits. In Rust, drop such a datagram.

### 3.2 Ack history (`update_received_packet_history`, `:560-576`)

```
per source: recent = deque(maxlen=64)                       # :1490
mask = 0
for prev in recent: d = (cur - prev) & 0x3FFF; if 1 <= d <= 32: mask |= 1 << (d-1)
recent.append(cur)        # after computing; duplicates are appended too
cur outside [0, 0x4000) -> ValueError
```
The mask is used as `ack_history_mask` on every send triggered by this datagram. The deque holds the last 64 **received** sequences, including duplicates. It is not a sliding window by sequence number.

---

## 4. Layer 3: bunch header

There are two readers with **different edge behavior**. Rust needs both, or one reader with both behaviors.

### 4.1 Field layout (both readers)

The reader starts at decoded bit 64: `has_packet_info = take(1)`; if set, `take(10)`. Then one more `take(1)` **unconditionally** (`range_udp_handshake.py:490-493`, `range_client_rpc.py:87-89`). The bunch begins after that:

| # | Field | Width | Condition |
|---|---|---|---|
| 1 | bControl | 1 | always |
| 2 | bOpen | 1 | if bControl |
| 3 | bClose | 1 | if bControl. If set, **stop** (see 4.3). No close reason is read. |
| 4 | bReplicationPaused | 1 | always |
| 5 | bReliable | 1 | always |
| 6 | ChIndex | packed uint (§5.1) | always |
| 7 | bHasPackageMapExports | 1 | always |
| 8 | bHasMustBeMappedGUIDs | 1 | always |
| 9 | bPartial | 1 | always |
| 10 | header_flag_4 | 1 | always (meaning unknown) |
| 11 | ChSequence | **12** | if bReliable |
| 12 | partial bits | 2 | if bPartial (skipped, meaning unknown) |
| 13 | bHardcodedName | 1 | if **bControl or bReliable** |
| 14 | ChName index | packed uint | if bHardcodedName. If the bit is 0, **nothing more is read** (no FString). |
| 15 | BunchDataBits | **13** | always |
| 16 | body | BunchDataBits | `body_start = position`, `end = body_start + BunchDataBits` |

`control_message_type` is the first 8 body bits when `BunchDataBits >= 8` (`range_udp_handshake.py:525`). It is computed for **every** first bunch, whatever the channel. It means NMT only on channel 0. That read goes through the same bounded `take`, so `body_start + 8 > P` (with `BunchDataBits >= 8`) raises, and the whole inspection becomes `bunch_parse_error`.

Observed values: client ch5 and ch8 reliable bunches carry `hardcoded_name=1, channel_name_index=102`. Unreliable bunches have no name and no sequence (header 28 bits: bunch body at 104 when the first bunch is at 76). The Hello is on ch0, control, open, reliable, with name 255 as the two packed bytes `FF 02`.

### 4.2 `inspect_control_bunch(decoded, P)` (`range_udp_handshake.py:476-538`): first bunch only

- `P < 128` returns `{}`. A 77-bit ack-only packet therefore has no `channel_index` and no `control_message_type`.
- Each `take` raises `ValueError("control header exceeds packet")` past `P`, and a packed value past 5 bytes raises `ValueError`. The caller turns both into `bunch_parse_error`.
- Close returns `{"bunch_start_bit", "bunch_is_close": True}` and nothing else.
- There is **no** check that `end <= P`.
- Returned keys (logged as the event): `bunch_start_bit`, `bunch_header_end_bit` (= body start), `bunch_is_control`, `bunch_is_open`, `bunch_replication_paused`, `bunch_reliable`, `channel_index`, `has_exports`, `has_must_map`, `bunch_partial`, `header_flag_4`, `channel_sequence` (None if unreliable), `channel_sequence_start_bit`, `hardcoded_name`, `channel_name_index` (None if no name), `bunch_bit_count`, `bunch_body_start_bit`, `bunch_end_bit`, `control_message_type` (None if fewer than 8 body bits).
- Used for: routing the input ack (`channel_index == 8`, body bounds), Login and Join detection, and capture logging.

### 4.3 `iter_bunches(decoded, P)` (`range_client_rpc.py:84-128`): all bunches

```
pos = 64; if take(1): take(10); take(1)
while pos + 16 < P:
    try: b = read_header()          # layout 4.1
    except ValueError: break        # truncated header or body
    if b is None: break             # control+close bunch: stop walking, bunch not returned
    append(b); pos = b.end_bit
```
- `read_header` raises if `body_start + BunchDataBits > P`. `end == P` is **allowed**: one captured ch8 bunch ends exactly at `P` (§13.4, `ch8_overrun`).
- A `Bunch` holds `start_bit, is_control, is_reliable, is_partial, channel_index, channel_sequence|None, body_start_bit, end_bit`.
- Partial bunches are returned, but both RPC decoders skip them (`is_partial` leads to None). Captured partials: two ch5 reliable (`responder_30456` @78.157 seq 445 and @90.515 seq 958, both P = 7656, body 127..7383) and one ch8 unreliable (`responder_32540` @81.141, body 106..7362). A reliable RPC split across partials is acked and never decoded.
- Used by the purchase and settings decoders. Across the 16923 captures: 16716 packets had 1 bunch, 189 had 2 and 18 had 3, for example `[ch5 rel 176][ch5 rel 177][ch8 unrel field 51]`. Client bunches seen: ch8 unrel 16711, ch5 unrel 224, ch5 rel 199, ch4 unrel 8, ch8 rel 5, ch6 rel 1.

### 4.4 Client reliable bunches: what Python does **not** do

- Python never tracks the client's `InReliable` per channel. It does not reorder or buffer out-of-order reliable bunches.
- Dedup is per decoder, by the raw 12-bit `ChSequence`, in a set that is never cleared (§6.5, §7.4).
- Every authenticated packet is acked at the packet level, decoded or not. **A reliable RPC that arrives while its decoder is gated off is acked and lost**: the client never resends it.

---

## 5. Layer 4: primitives, content block, content transform

### 5.1 Primitives

- **Packed uint** (`read_packed_uint`, `range_client_input.py:25-35`; `_packed_header_field`, `range_client_rpc.py:74-81`):
  - Up to 5 bytes. Each byte gives `value |= (byte >> 1) << (7*i)` and continues while `byte & 1`.
  - A 6th byte raises `ValueError("…exceeds five bytes")`, and so does a 5th byte with bit 0 set.
  - The result is **not** masked to 32 bits: up to 35 bits, so `fffffffffe` gives 34359738367.
  - `read_packed_uint` raises "truncated" when `pos + 8 > limit`.
- **SerializeInt(max)** (`_Reader.serialize_int`, `range_client_rpc.py:54-61`):
  ```
  value = 0, mask = 1
  while value + mask < max and mask: if take(1): value |= mask; mask <<= 1
  ```
  The width depends on the value **and on `max`**. With max 179: 156 → 8 bits, 70 → 7 bits, 83 → 7 bits, 92 → 7 bits. For the pawn, max is 54 (class max 53 + 1, the same convention the server writer uses: `append_serialize_int(field, class_max + 1)`). 51, 41 and 25 happen to have the same width under 53 or 54, but 21 does not: the bits `1,0,1,0,1,1` read as 21 (5 bits) with max 53 and as 53 (6 bits) with max 54.
- **FString** (`range_client_rpc.py:63-71`):
  - `len = take(32)`.
  - If `len & 0x80000000`: UTF-16, `n = 2^32 - len`, read `2n` bytes, decode utf-16-le.
  - Otherwise: if `len > 1024`, `ValueError`; read `len` bytes and decode ASCII. A non-ASCII byte raises `UnicodeDecodeError`, which is a `ValueError`.
  - Both paths then `rstrip("\0")`, stripping **all** trailing NULs. `len == 0` gives `""`.
  - The UTF-16 length has no bound beyond running out of bits.
- `_Reader.take(w)` raises `ValueError("read past end")` when `pos + w > limit`.

### 5.2 Content block header (`read_actor_content_block`, `range_client_input.py:150-167`)

At the bunch body start:

```
has_rep_layout = bit(body+0)
is_actor       = bit(body+1)      # 0 -> ValueError("subobject block needs its own NetGUID context")
bit_count      = packed uint at body+2      (limit = body_end)
bit_offset     = position after the packed uint
reject if bit_count > 8192 or bit_offset + bit_count > body_end
wire = ceil(bit_count/8) bytes copied from bit_offset (last byte partial)
payload = decode_content_payload(wire, bit_count, actor_map_value)
```
- Every reject is a `ValueError`. The full list, in check order:
  - `body_start < 0`, `body_end > len(packet)*8` (the bound is the **byte** length, not `P`) or `body_start >= body_end`: "invalid actor bunch bounds"
  - `body_start + 2 > body_end`: "actor block header truncated"
  - `is_actor == 0`: subobject
  - truncated packed `bit_count`
  - `bit_count > 8192` or overrun: "actor content length exceeds bunch"
  - `bit_count == 0`: "invalid content payload length", from `decode_content_payload`
- Only the **first** content block of the bunch is read. Python does not check whether the block fills the bunch.
- `actor_map_value` is the NetGUID of the channel's actor: **ch5 controller = 4** (`range_client_rpc.py:24`), **ch8 pawn = 10** (the default argument).
- Subobject blocks (`is_actor = 0`) are never decoded. The captures hold 4 ch5-reliable and 5 ch8-reliable subobject bunches (examples in §13.4). Python skips them.

### 5.3 Content transform (`decode_content_payload`, `range_client_input.py:93-138`)

The input requires `1 <= bit_count <= 8192` and `len(wire) == ceil(bit_count/8)`. The stage structure is the same as §2.3, with a different state machine and stage functions:

```
seed = (actor_map_value ^ bit_count) & M32
mix(v, src) = (MUL * (v ^ ((src << 25) & M32) ^ ((v ^ (v >> 15)) >> 12))) & M64      # v, src are u32
first  = mix(seed, seed)
second = mix((seed - 378250948) & M32, (seed + 60) & M32)
advance():  ns = ((first + second) & M64) >> 32
            comb = second ^ first
            second' = (comb ^ (comb << 14) ^ rotr64(second, 9)) & M64
            first'  = rotr64(comb, 28)
            seed = ns
stages in order (advance after each):
  bit_count/64 times:            out64 = dec_wide(wire_u64_le, seed)
  if remaining bits >= 32, once: out32 = dec_word(wire_u32_le, seed)
  each remaining whole byte:     out8  = dec_byte(wire_byte, seed)
  final partial (r = bit_count%8 > 0), no advance needed:
                                 out = (wire[-1] ^ (seed ^ 0x3C)) & ((1<<r)-1)
```
Stage functions, where the inverse tables are equal to `wide_sbox_inverse` and `word_sbox_inverse` (checked) and `byte_sbox_inverse`:

```
swap64(v) = ((v>>1) & 0x5555555555555555) | ((v<<1) & 0xAAAAAAAAAAAAAAAA)       (& M64)
dec_wide(w, s):                                        # :65-71
  m = swap64(w) ^ ((~rotr32(s,7)) & M64)   # NOTE: ~ of a u32 zero-extended -> upper 32 bits all 1
  x = rev_wide(swap64(m))                  # rev_wide from 2.3
  x = S⁻¹w(x)
  x = (rotr32(s,2) - 1 - x) & M64          # u32 rotate, zero-extended, 64-bit wrapping sub
  return S⁻¹w(x)
dec_word(w, s):                                        # :74-79
  x = scramble_content_word(w, s)          # range_server_bunch.py:1030-1047, below
  x = S⁻¹d(x)
  x = (rotl32(s,2) - 1 - x) & M32
  return S⁻¹d(x)
dec_byte(b, s):                                        # :82-90
  f  = ((195*s) & 0xFF) ^ (((b>>1) & 0x55) | ((2*(b & 0xD5)) & 0xFF))     # -61 ≡ 195 mod 256
  sh = ((0x10101 * (((32800*f) & 0x88440) | ((2050*f) & 0x22110))) >> 16) & 0xFF
  ii = ((sh>>1) & 0x55) | ((2*(sh & 0xD5)) & 0xFF)
  inner = S⁻¹b[ii]
  return S⁻¹b[((121*s) + (0xFF - inner)) & 0xFF]      # Python: (121*s + ~inner) & 0xFF
scramble_content_word(w, s):     # use u64 intermediates
  sw(v) = ((2*(v & 0xD5555555)) | ((v>>1) & 0x55555555)) & M32
  m = sw(sw(w) ^ rotl32(s,7))
  high = ((m>>2) & 0x33030303) | ((4*(m & 0xF3333333)) & 0xFF0F0F0F)
  low  = (((m>>2) & 0x33333333) | (4*(m & 0xF3333333))) >> 4
  folded = ((16*high) | (low & 0x0F0F0F0F)) & M32
  return sw(byteswap32(folded))
```
The client's writer is the inverse of this reader. Python's server-side `encode_short_actor_rep_payload` (`range_server_bunch.py:1131`) is that inverse, and the test `test_content_transform_roundtrip` pins the pair. Vectors are in §13.6.

### 5.4 RPC field header (inside the decoded payload)

```
field         = SerializeInt(class_max + 1)
payload_bits  = packed uint
params        = the next payload_bits bits     # params reader limit = pos + payload_bits (not clamped to bit_count)
```
- **Controller** (ShootingRangeController_C, ch5): `controller_field_max = 179` (`range_client_rpc.py:23`). This is the live FClassNetCache max 178, plus 1 (`:197`; `range_server_bunch.py:929-933`).
- **Pawn** (ch8): class max 53, so the field is `SerializeInt(·, 54)`. This matches the server-to-client pawn RPCs: `move_ack_class_max = 53` and input ack field 41 of 53, both written with `class_max + 1` (`launch_range.py:50`, `range_udp_handshake.py:1541-1542`). Python never decodes a ch8 field index (§8).

Fields seen in non-partial, actor-level, non-rep-layout client ch5 bunches across the 16923 captures. Only 156 and 176 are acted on:

| Field (max 179) | Reliable | Params bits seen (count) | Python |
|---|---|---|---|
| 156 ServerPurchase | yes | 523 (armor) and 691-843 (guns), full path; 171, 203, 243, 251 in the `<guid:N>` form (§6.2) | §6 |
| 176 ServerUpdateShootingRangeSettings | yes | 133 (16) | §7 |
| 177 ServerUpdateTargetPracticeSettings | yes | 3 (16) | §7, logged only |
| 70 | yes | 9 (59) | ignored; identity not established |
| 83 | **no** (unreliable ch5) | 197 (197), 229 (27) | ignored; identity not established |
| 92 | yes | 574 (16), 398 (16) | ignored; identity not established |
| 154 | yes | 5 (1), 149 (2) | ignored; identity not established |

---

## 6. ServerPurchase family (`range_client_rpc.py:131-194`)

### 6.1 Gate per bunch (`decode_controller_rpc`, `:160-178`)

Return None unless `channel_index == 5 && is_reliable && !is_partial`. Read the content block with `actor_map_value = 4`. Return None if `has_rep_layout`. Read the field with `serialize_int(179)` and return None unless it is in `{156, 157, 161}`. Then:

```
payload_bits = packed
params (limit = start + payload_bits):
  ItemToPurchase  : present = take(1); item_path = object_reference() if present else None
  TargetSlot      : present = take(1); slot = take(8) if present else 0      # EAresItemSlot
  Type            : present = take(1); type = take(3) if present else 0      # EAresPurchasableType
→ PurchaseRequest(field, item_path, target_slot, purchasable_type, bunch.channel_sequence)
```
- There is **no** check that the params end at `payload_bits`. The captured Warden params end exactly (737 + 2 = 739 = limit), and so does every other captured purchase.
- In the captures, TargetSlot is **present** for sidearms (slot 1) and armor (slot 11), and absent (decoded 0) for primaries. Type is never present (decoded 0). `target_slot` and `purchasable_type` are logged only (`range_udp_handshake.py:1869-1870`). The slot used comes from the store table (§6.6).

### 6.2 Object reference (`_object_reference`, `:131-148`)

```
ref(depth):
  if depth > 8: ValueError
  g = packed
  g == 0  -> None                          (null)
  g != 1  -> "<guid:g>"                    (mapped GUID, nothing else follows)
  g == 1  (default GUID; client-originated path follows if flagged):
     flags = take(8)
     if !(flags & 1): -> "<guid:1>"        (bHasPath unset)
     outer = ref(depth+1)
     name  = FString
     if flags & 4: take(32)                (network checksum; discarded)
     -> f"{outer}.{name}" if outer else name
```
String forms the Rust decoder must produce identically:

| Form | Meaning |
|---|---|
| `None` | presence bit 0, or NetGUID 0 |
| `"<guid:N>"` | mapped GUID N (N ≥ 2) |
| `"<guid:1>"` | default GUID with no path |
| `"Name"` | path whose outer is null |
| `"/Game/Pkg/X.X_C"` | path with an outer path (outer is the package, null outer) |
| `"<guid:N>.X_C"` | path whose outer is a GUID the client already has mapped |
| `"A.B.C"` | deeper outer chains (rejected later by `split_class_path`) |

Captured first purchase (Warden), traced (bit positions are relative to the decoded payload):

```
0    field 156 (8 bits) ; packed payload size 715 (16 bits: 0x97 0x0a)
24   ItemToPurchase present = 1
25   NetGUID 1 (8)   33 flags 0x05 (bHasPath|bHasChecksum)
41     NetGUID 1 (8) 49 flags 0x05
57       NetGUID 0 (8)                  -> outer of package = null
65     FString '/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle' (464 bits)
529    checksum 0x00000000
561  FString 'BattleRifle_C' (144 bits)
705  checksum 0xb77e876f
737  TargetSlot present 0 ; 738 Type present 0 ; end 739 == limit
```
The ACR purchase has the same shape, with the package string 472 bits and the class 184 bits, and the same checksums `0` / `0xb77e876f`.

The repeat-purchase form `"<guid:525>.AssaultRifle_AK_C"` (`range_store.py:67-68`, handled by `resolve_class_path`) **is in the captures**: 22 bunches, all from `responder_20928`. The real client sends it with **flags 0x05** (path and checksum), not 0x01:

```
0    field 156 (8) ; packed payload size 243 (16 bits: 0xe7 0x02)
24   ItemToPurchase present = 1
25   NetGUID 1 (8)   33 flags 0x05
41     NetGUID 525 (16: 0x1b 0x08)        -> outer = "<guid:525>", nothing else follows for it
57   FString 'AssaultRifle_AK_C' (176 bits)
233  checksum 0xb77e876f
265  TargetSlot present 0 ; 266 Type present 0 ; end 267 == limit
```
The outer GUIDs seen (525, 555, 583, 599, 611, 629, 681, 683, 829) are all odd, which fits package GUIDs handed out by `PurchaseAllocator.static_pair` (§6.5). The real vectors are in §13.5. The 0x01 (no-checksum) synthetic vector is kept there too, because Python decodes both.

### 6.3 Packet-level walk (`decode_purchase_requests`, `:181-194`)

`iter_bunches`, then `decode_controller_rpc` for each bunch. A `ValueError` from one bunch skips **that bunch** only (that covers `UnicodeDecodeError` from an FString too). `iter_bunches` catches header errors inside its loop, but its three pre-loop `take`s (bits 64..75) are outside the `try`. With `P < 76` they raise, and `decode_purchase_requests` returns `[]`.

### 6.4 Constants

`server_purchase_field = 156`, `server_remove_field = 157`, `server_sell_field = 161`, `controller_field_max = 179`, `controller_actor_map_value = 4` (`range_client_rpc.py:20-24`).

### 6.5 Responder use (`range_udp_handshake.py:1851-1907`)

Gate: the authority block (§1), and `store_purchases`, and `starting_equippable_index.get(source, 0) >= len(wushu_starting_equippables)` (= 7; `range_server_bunch.wushu_starting_equippables`, imported at `:879`). The whole datagram is decoded again from scratch: `decode_client_ares_packet` then `decode_purchase_requests`. Any exception sets `event["purchase_decode_error"] = repr(error)[:200]` and gives no purchases. The per-datagram job advance (step 9) runs whenever this gate is open, whether or not the packet held a purchase.

For each request, in order:
1. `seen = purchase_sequences_seen[source]`. If `channel_sequence in seen`, skip. Otherwise add it **before** any other check. 157/161/pathless requests consume their sequence too. The set is shared by 156, 157 and 161 and is never cleared, and it holds the raw 12-bit value, so it collides after a wrap.
2. Log `received_purchases += [field, item_path, slot, type]`.
3. Continue unless `field == 156 and item_path` is truthy.
4. `class_path = resolve_class_path(item_path, purchase_package_guids[source], purchase_seen_paths[source])` (§6.6).
5. `item = store_item_for_path(class_path)` when `class_path` is set. A `ValueError` gives None. If None, log `purchase_unsupported = item_path` and continue.
6. If `class_path != item_path`, log `purchase_resolved_path`.
7. `purchase_seen_paths[source][item.class_name] = class_path`.
8. `allocator = purchase_allocators[source]` (`range_gun_loader.PurchaseAllocator`: channels from 20, NetGUIDs from 520; `range_gun_loader.py:38-70`).
   - Non-gun (armor): `plan_item_load`, then `purchase_package_guids[source][load.package_guid] = load.package`, then queue the job with `"armor": True`.
   - Gun: `plan_gun_load`, then the same package-GUID registration, then queue the job.
   - Log `purchase_planned = [display_name, channel, actor_guid]`.
9. Afterwards, every job with `stage < 7 and now >= job["next"]` is advanced. That belongs to the gun-loader doc.

Every purchase allocates a **new** package GUID, even for a gun bought before. `purchase_package_guids` keeps all of them.

### 6.6 Class path resolution (`range_store.py:53-90`)

```
resolve_class_path(item_path, package_guids, seen_paths):        # :63-85
  if not item_path: None
  if not item_path.startswith("<guid:"): return item_path        # full path passes through unvalidated
  head, _, class_name = item_path.partition(">.")
  guid = int(head[6:].rstrip(">"))   # ValueError -> None
  if class_name and guid in package_guids: return f"{package_guids[guid]}.{class_name}"
  if class_name and class_name in seen_paths: return seen_paths[class_name]
  return None
split_class_path(p):                                               # :53-60
  require str, startswith "/Game/", exactly one "."
  package, class_name = p.split(".")
  require class_name.endswith("_C") and package.endswith("/" + class_name[:-2])
  -> (package, class_name, "Default__" + class_name)
store_item_for_path(p) = store_items.get(split_class_path(p)[1])  # may raise ValueError
```
Store table (`range_store.py:25-49`). Slots: primary 0, secondary 1, armor 11 (`:12`).

| Display | Class | Cost | Slot | Gun | UUID |
|---|---|---|---|---|---|
| Classic | BasePistol_C | 0 | 1 | y | 29a0cfab-485b-f5d5-779a-b59f85e204a8 |
| Shorty | SawedOffShotgun_C | 300 | 1 | y | 42da8ccc-40d5-affc-beec-15aa47b42eda |
| Frenzy | AutomaticPistol_C | 450 | 1 | y | 44d4e95c-4157-0037-81b2-17841bf2e8e3 |
| Ghost | LugerPistol_C | 500 | 1 | y | 1baa85b4-4c70-1284-64bb-6481dfc3bb4e |
| Bandit | CompactPistol_C | 600 | 1 | y | 410b2e0b-4ceb-1321-1727-20858f7f3477 |
| Sheriff | RevolverPistol_C | 800 | 1 | y | e336c6b8-418d-9340-d77f-7a9e4cfe0702 |
| Stinger | Vector_C | 1100 | 0 | y | f7e1b454-4ad4-1063-ec0a-159e56b58941 |
| Spectre | SubMachineGun_MP5_C | 1600 | 0 | y | 462080d1-4035-2937-7c09-27aa2a5c27a7 |
| Bucky | PumpShotgun_C | 850 | 0 | y | 910be174-449b-c412-ab22-d0873436b21b |
| Judge | AutomaticShotgun_C | 1850 | 0 | y | ec845bf4-4f79-ddda-a3da-0db3774b2794 |
| Bulldog | AssaultRifle_Burst_C | 2050 | 0 | y | ae3de142-4d85-2547-dd26-4e90bed35cf7 |
| Guardian | DMR_C | 2250 | 0 | y | 4ade7faa-4cf1-8376-95ef-39884480959b |
| Phantom | AssaultRifle_ACR_C | 2900 | 0 | y | ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a |
| Vandal | AssaultRifle_AK_C | 2900 | 0 | y | 9c82e19d-4575-0200-1a81-3eacf00cf872 |
| Warden | BattleRifle_C | 2900 | 0 | y | 8db0a1bf-4a50-832a-4566-faaaa6d250ca |
| Marshal | LeverSniperRifle_C | 950 | 0 | y | c4883e50-4494-202c-3ec3-6b8a9284f00b |
| Outlaw | DS_Gun_C | 2400 | 0 | y | 5f0aaf7a-4289-3998-d5ff-eb9a5cf7ef5c |
| Operator | BoltSniper_C | 4700 | 0 | y | a03b24d3-4319-996d-0f8c-94bbfba1dfc7 |
| Ares | LightMachineGun_C | 1600 | 0 | y | 55d8a0f4-4274-ca67-fe2c-06ab45efdf58 |
| Odin | HeavyMachineGun_C | 3200 | 0 | y | 63e6c2b6-4a8e-869c-3d4c-e38355226584 |
| Light Shields | LightArmorItem_C | 400 | 11 | n | 4dec83d5-4902-9ab3-bed6-a7a390761157 |
| Heavy Shields | HeavyArmorItem_C | 1000 | 11 | n | 822bcab2-40a2-324e-c137-e09195ad7692 |
| Regen Shield | PlasmaArmorItem_C | 650 | 11 | n | b1b9086d-41bd-a516-5d29-e3b34a6f1644 |

---

## 7. Range settings RPCs (`range_client_rpc.py:197-292`)

### 7.1 Field constants (ShootingRangeController_C, max 178, read with `serialize_int(179)`)

| Field | Name | Direction |
|---|---|---|
| 170 | ClientUpdateRangeSettings (no params) | server to client (the request; built by `build_client_update_range_settings_payload`, `range_server_bunch.py:927-935`) |
| 172 | ServerEndMissionModule | client to server |
| 173 | ServerLoadAndSwapCharacter | client to server |
| 174 | ServerResetBots | client to server |
| 175 | ServerToggleFreeShooting | client to server |
| 176 | ServerUpdateShootingRangeSettings(FShootingRangeSettings) | client to server (reply) |
| 177 | ServerUpdateTargetPracticeSettings(int32, bool, bool) | client to server (reply) |

### 7.2 Decode (`decode_range_settings_rpc`, `:233-276`)

The gate is the same as purchases: ch5, reliable, not partial, `actor_map 4`, not rep-layout, field in `{172..177}`. Rule (`:236-237`): "one presence bit per non-bool parent parameter; a struct parameter's members follow in order with no handles; bools are 1 bit".

```
174, 175 : payload_bits must be 0, otherwise ValueError("parameterless settings RPC carried parameters")
176      : present = take(1)
           if !present: all defaults
           else: bot_armor 1, bot_strafe 1, infinite_ammo 1,
                 skill_test i32, plant_difficulty i32, defuse_difficulty i32,
                 practice_mode 1, bot_amount_index i32          (total 1+3+96+1+32 = 133)
177      : distance_index = i32 if take(1) else 0 ; bot_enabled = take(1) ; is_init_settings_flow = take(1)
172, 173 : object_path = object_reference() if take(1) else None
then all of them except 174/175: params.position must == start + payload_bits, otherwise ValueError
```
`i32` means `take(32)` reinterpreted as signed two's complement (`_int32`, `:229-230`). `RangeSettingsRequest` fields and defaults are at `:208-226`. `channel_sequence` is taken from the bunch.

`decode_range_settings_requests` (`:279-292`) walks bunches the same way purchases do (§6.3).

### 7.3 Captured reply (one packet, 3 bunches)

Decoded payloads:
- 176 `b00b020900000000000000000000002000000000` (157 bits). Field `0xb0`, size packed `0b 02` = 133, then `0x09` = present 1, armor 0, strafe 0, ammo 1, … The result: `infinite_ammo=True, bot_amount_index=1`, everything else 0 or False.
- 177 `b10604` (19 bits). Field `0xb1`, size 3, then bits `0,0,1`: distance absent, bot_enabled 0, is_init_settings_flow 1.

### 7.4 Responder use (`range_udp_handshake.py:1913-1946`)

Gate: the authority block, and `range_settings`, and `source in range_streaming_sent_sources`.

1. **Request, once.** If `source not in range_settings_requested` and `now - range_hud_sent_times.get(source, now) >= 3.0` (`:1920-1921`; a missing entry never fires), send `send_reliable(5, build_controller_rpc_body(build_client_update_range_settings_payload()), has_exports=False)`. Add `source` to `range_settings_requested` and log `sent_client_update_range_settings`.
2. Decode the whole datagram again: `decode_client_ares_packet` then `decode_range_settings_requests`. Any exception sets `range_settings_decode_error = repr(error)[:200]` and gives no requests.
3. For each request: dedup through `range_settings_sequences_seen[source]` (raw 12-bit sequence; a set shared by 172-177, never cleared; separate from the purchase set). Log `received_range_settings += {k: v for k, v in request.__dict__ if v}`, keeping truthy fields only, in dataclass field order. A falsy `channel_sequence` (0) is dropped from the dict too, and so are `distance_index` 0, `False` bools and `object_path` None. Example: the synthetic seq-0 field-172 request logs `{"field": 172, "object_path": "/Game/X/Mod"}`.
4. For field 176:
   - `range_settings_state[source] = request`.
   - If `range_infinite_ammo_sent.get(source) != request.infinite_ammo`, send `send_reliable(4, build_game_state_bool_body(46, infinite_ammo), False)` and set `range_infinite_ammo_sent[source]` (handle 46 is `game_info_infinite_ammo_handle`, `range_server_bunch.py:924`).
   - The first 176 always sends, because `None != False`.
5. Every other field is logged only.

---

## 8. Channel 8 (pawn): input-event streams and movement

### 8.1 What the ch8 client bunch is

- In 16691 of 16691 captured non-rep-layout, actor-level ch8 blocks, the client's **unreliable** ch8 bunch carries pawn RPC **field 51** (`SerializeInt(51, 54)` = 6 bits), and the packed size exactly fills the block.
- **Python never reads this field index.** It finds the input stream by a bit-aligned scan of the decoded content for marker `0xA3`.
- Python does **not** decode any movement data from ch8. The move ack takes its numbers from client memory (`read_move_number.read_saved_moves`). The movement fields inside RPC 51 are unspecified here because Python never parses them.
- Other observed ch8 client blocks, all ignored:
  - unreliable rep-layout blocks (13)
  - ch8 reliable subobject blocks (5)
  - unreliable blocks whose length overruns the bunch (7)
  - one unreliable partial bunch (§4.3)

### 8.2 Stream wire format (`_parse_input_stream_at`, `range_client_input.py:190-245`)

Offsets are relative to the marker bit inside the decoded content payload:

```
+0   8   marker 0xA3
+8   1   must be 1
+9   32  first_event_id (u32)
+41  packed count; require 1 <= count <= 256 and first+count-1 <= 0xFFFFFFFF
     if max_generated_event_id given: require last_event_id <= it
then count events:
     1 bit must be 1 ; kind = 4 bits ; then payload by kind:
       kind 4, 8 : packed uint object_guid
       fixed bits: {0:37, 1:11, 2:1, 3:2, 5:48, 6:32, 7:32, 9:40, 10:9, 11:8, 12:32}
       kinds 13, 14, 15 (not in table) -> reject the whole stream
       kind 1 payload (11 bits, e): flag = e&1, slot = (e>>1)&0xF, action = (e>>5)&7, state = (e>>8)&7
after the events: position < bit_count and next bit == 0 (terminal), end_bit = position + 1
```
- Any failure gives no stream at this marker: a truncated field, a bad continuation bit, an unknown kind, or a missing terminal.
- The precondition is `marker_bit + 49 <= bit_count` (`:198`).
- Native references from the docstring: RVA 0x6D34950 writes the marker, 0x6D41F90 writes the bounds and count, 0x6D41380 writes the kind and payload.
- Event IDs are `first_event_id + index`.

### 8.3 Scan (`scan_received_input_streams`, `:248-269`)

- Every `marker_bit` in `range(max(0, bit_count - 48))` whose 8 bits equal `0xA3` is tried, and every successful parse is returned.
- **The scan does not skip past a stream it has found.** Spurious "streams" inside a real stream's events are returned too.
- Captured example (§13.7): a real stream at bit 315 with 41 events, plus 3 spurious kind-8 single-event streams at bits 497, 693 and 746, with first IDs around 3.6e9.

### 8.4 `decode_received_actor_inputs(packet, body_start, body_end, max_id=None)` (`:272-281`)

1. `read_actor_content_block(..., actor_map_value=10)`.
2. If `has_rep_layout`, return `()`.
3. Otherwise return `scan_received_input_streams(payload, bit_count, max_id)`.

Marker positions of single streams in the captures: 315 (649 cases), 1014 (237), 1247 (137), 323 (25), 762 (9), then 2645, 3810, 252 and others. Across 16524 first-bunch-ch8 packets:
- 15434 had 0 streams, 12 of them because the content block raised.
- 1087 had 1.
- 2 had 2. Both were a real stream at 315 plus a spurious single-event stream at 325.
- 1 had 4 (§13.7 D).

A single stream can be spurious. `responder_24064` @139.297 (P = 5610) yields exactly one stream: marker 3099, one kind-11 event, id 3792864112. Only the RPM bound in §8.5 keeps Python from acking it.

### 8.5 Input ack (`range_udp_handshake.py:1499-1563`)

Gate:
- `input_ack_cursor_trial`
- `source in client_restart_sent_sources`
- receive key known
- **the first bunch's** `channel_index == 8`, from `inspect_control_bunch`

A ch8 bunch that is not first in its packet is ignored, for example `[ch5][ch8]`. `bunch_partial` and `bunch_reliable` are **not** checked: a partial or reliable first ch8 bunch goes through the same decode. In the captures it always raised and produced no stream.

1. `streams = decode_received_actor_inputs(decoded, event.bunch_body_start_bit, event.bunch_end_bit)` with **no** max ID. `KeyError` or `ValueError` gives `()`. The bounds come from `inspect_control_bunch`, so `end` may exceed `P`. Only `end > len(decoded)*8` is rejected (§5.2).
2. If `len(streams) == 1`:
   - Call `read_pending_input_bounds(pid, image_base)`. This is **RPM**. The pointer chain is GObjects object `0x107E2` (engine) `+0xBD0` viewport `+0x80` world `+0x1D8` game instance `+0x40` → `[0]` local player `+0x38` controller `+0x4B0` pawn `+0xB90` input component. The component's cursor is at `+0xA84` (u32) and its count at `+0x1198` (u32); `latest_generated = (cursor + count) & M32` (`capture_input_event_cursor.py:23-63`). A null pawn or component returns None, and `count > 4096` raises `ValueError`. `OSError`, `ValueError` and `struct.error` are caught as None.
   - If the bounds are None, or `last_event_id > latest_generated`, log `input_ack_rejected_event_id` and set `streams = ()`.
3. If `len(streams) == 1`:
   - Log `received_activation_events` (the kind-1 events: id, slot, state, action).
   - If `last_event_id > last_input_ack_numbers.get(source, -1)`:
     - Send an **unreliable ch8** bunch carrying `build_input_ack_rpc_payload(last_event_id, 0, 41, 53)` (field `SerializeInt(41, 54)`), encoded with actor map 10, at `next_server_sequences[source]`. `BunchHeader` is given `channel_name_index=102`, but an unreliable bunch puts no name and no ChSequence on the wire (`range_server_bunch.py:1588-1602`). The builder is in the server-builder doc; a body vector is in §13.9.
     - Increment the server sequence.
     - Set `last_input_ack_numbers[source] = last_event_id`.
     - Log `input_ack_received_event_id` and `input_ack_received_frame_length`.
4. When there are 0 or 2+ streams, nothing is sent for this packet.

### 8.6 Capture logging (`:2086-2106`), production, logging only

- **First bunch on ch8.** Applies when `source in pawn_role_sent_sources`, `movement_capture_counts[source] < 300`, and `elapsed - movement_capture_last_times[source] >= 0.25` (default -1.0). Decode again, then log `decoded_packet_hex`, and update the count and the last time.
- **First bunch on ch5** (after the pawn role). The key is `("reliable", channel_sequence)` or `("unreliable", int(elapsed_seconds))`. If the key is new and fewer than 16 keys are stored, log `decoded_packet_hex`.

These logs are the source of the real-traffic fixtures used in this doc. Rust should keep the same keys at debug level (§11).

---

## 9. Control channel recognition (the `elif` chain, `range_udp_handshake.py:2107-2184`)

The branches are evaluated in this order. Only the first match runs.

1. **Not authenticated.** Set `event["parse_error"] = str(handshake error)`.
2. **Hello capture.** If `payload_bits == 232 and source in pending`, store `hello_packets[source] = decode_client_ares_packet(...)`.
   - It is identified by **bit count only**. The client **retransmits** the Hello: `responder_24252` logged 5 copies (ch0 seq 3528, history word 0) between @60.891 and @65.907, before and after the Challenge. Every copy overwrites `hello_packets[source]`.
   - `pending` is never cleared, so **any** later authenticated 232-bit packet from this source is stored as "hello". For that datagram this skips the session-ack branch and everything nested in it: the setup stage chain and the move ack (`:4412`, inside the session-ack branch).
   - Before the Welcome is sent, the stored "hello" is the template `build_server_control_probe` copies. If a non-Hello 232-bit packet overwrote it, that builder raises an **uncaught** `ValueError` (`:649-653`: type ≠ 0, channel ≠ 0, not control, not reliable, or `bunch_end_bit ≠ 231`) and the responder dies. Hello retransmits carry the same bunch (ChSequence 3528). Their logged prefixes (`ff3f0100`, `693e0100`, `ad3e0100`, `ef3e0100`, `313f0100` at bytes 8-11) differ only in the packet-info bits 65..74, which lie outside the copied region (bits 76 onward). So they are harmless.
3. **Challenge.** Condition: `source not in ack_sent_sources and source in pending and payload_bits == 77`.
   - If `source in hello_packets`, build `build_server_challenge_probe(hello, 232, this, 77, stateless_header, receive_key)`. Its `server_sequence = (((H >> 4) & 0x3FFF) + 1) & 0x3FFF`, from the client's acked field of **this 77-bit packet** (`:654-656`).
   - Set `next_server_sequences = (server_sequence + 1) & 0x3FFF` and `initial_actor_channel_sequences = (server_sequence + 1) & 0xFFF`, and add the source to `ack_sent_sources` (`:2140-2142`).
   - Otherwise `continue`. The datagram is then not even logged.
   - Before the send, `inspect_packet_notify` is read from client memory (RPM) for logging only (`pre_control_sequence`, `pre_in_sequence` or `pre_state_error`). Failure is tolerated. It is slow: it took 4797 ms in `responder_24252` @61.110 (`handler_elapsed_ms`). Log vector from that run: the 77-bit packet with prefix `307821b7000000002998` has H = `0xb7217830`, acked 6019, so `server_sequence` = 6020, and `pre_in_sequence` was 6019.
4. **Welcome.** Condition: `source in ack_sent_sources and not in welcome_sent_sources and in pending and control_message_type == 5`. The channel is **not** checked. `build_server_welcome_probe` itself re-checks type 5. It uses `server_sequence = next_server_sequences[source]` and history word 7. In the same run the Login was P = 1694 on ch0 seq 3529, and the Welcome went out with server sequence 6021.
5. **Join ack.** Condition: `source in welcome_sent_sources and not in join_ack_sent_sources and in pending and control_message_type == 9`. `build_server_join_ack` raises an **uncaught** `ValueError` unless the first bunch is ch0, reliable, type 9 (`:604-606`). That would kill the responder. Rust: treat it as "not a join" (see open questions). The join ack carries history word `3` (not `ack_history_mask`). It also sets `last_session_ack_times[source] = now` (`:2171`), so the first session ack needs another ≥ 0.1 s. Log check: Join (P = 142, ch0 seq 3531) at @71.688 got the join ack with server sequence 6022. The 77-bit packet at @71.735 got nothing; the one at @71.969 got the first session ack, 6023.
6. **Session ack.** Condition: `source in join_ack_sent_sources and in pending and now - last_session_ack_times[source] >= 0.1`. Sends the session ack with this datagram's `ack_history_mask`, then runs the setup stage chain and the move ack (other docs), all nested in this branch.

Synthetic control packets (from `test_range_udp_handshake.py:260-330`) are in §13.3.

---

## 10. Cross-cutting state touched by decoding

Every entry below is per source unless marked otherwise.

| State | Python | Init | Written | Read |
|---|---|---|---|---|
| `client_send_key`, `receive_key_override` | process-global | None | first datagram after start (§1) | every decode and send |
| `pending[source]` | `(timestamp, cookie)` | `:773` | handshake initial `:4547` | Hello, Challenge, Welcome, Join, session ack gates (never removed) |
| `hello_packets[source]` | decoded tuple | `:774` | `:2110` | Challenge builder |
| `recent_client_sequences[source]` | `deque(maxlen=64)` | `:1362` | `:1490-1492` | ack mask |
| `latest_authenticated_packets[source]` | `(seq, sh, mask, t)` | `:1363` | `:1493` | heartbeat (off) |
| `last_input_ack_numbers[source]` | int | `:1361` | `:1563` | `:1540` |
| `next_server_sequences[source]` | 14-bit | Challenge (set to client-acked + 2, `:2140`) | every later send (+1 & 0x3FFF) | every send except the Challenge |
| `ack_sent_sources` / `welcome_sent_sources` / `join_ack_sent_sources` | sets | `:804-806` | `:2142`, `:2157`, `:2170` | §9 gates |
| `last_session_ack_times[source]` | monotonic float | `:1408` | join ack `:2171`, session ack `:2184` | session-ack gate (default 0) |
| `purchase_jobs[source]` | list of loader jobs | `:920` | `:1894, 1901` | advanced inside the purchase gate every datagram (`:1904-1907`) |
| `initial_actor_channel_sequences[source]` | 12-bit | Challenge | — | `take_channel_sequence` |
| `purchase_sequences_seen[source]` | set of 12-bit sequences | `:904` | `:1867` | `:1865` |
| `purchase_package_guids[source]` | `{guid: package}` | `:924` | `:1893, 1900` | `resolve_class_path` |
| `purchase_seen_paths[source]` | `{class_name: path}` | `:925` | `:1888` | `resolve_class_path` |
| `purchase_allocators[source]` | `PurchaseAllocator` | `:919` | `:1889` | loader |
| `range_settings_requested` | set | `:936` | `:1924` | `:1920` |
| `range_settings_sequences_seen[source]` | set | `:937` | `:1937` | `:1935` |
| `range_settings_state[source]` | last 176 request | `:938` | `:1941` | none in production |
| `range_infinite_ammo_sent[source]` | bool | `:939` | `:1945` | `:1942` |
| `movement_capture_counts` / `_last_times` / `input_capture_sequences` | logging | `:867-869` | `:2093-2094, 2106` | same |

Gates owned by other stages: `client_restart_sent_sources`, `pawn_role_sent_sources`, `starting_equippable_index` (purchase gate needs `>= 7` = `len(wushu_starting_equippables)`), `range_streaming_sent_sources` and `range_hud_sent_times`.

---

## 11. Rust integration notes

- **Tick.** The server runs at 128 tps. Run the per-datagram pipeline (§1) for each received datagram in order inside the tick. Sends the pipeline emits go out in the same order within that tick, so server sequence numbers match Python's. RPM work (key scan, input bounds, packet-notify) must not block the tick (§1).
- **Console logging.** The server allocates a console (Windows `AllocConsole`) and logs one line per datagram event: Python prints `json.dumps(event)` per datagram, flushed (`:1376`). Python also prints `listening on 127.0.0.1:{port}` at bind and the handler/fingerprint line on key acquisition (§1). Use Python's event keys so the Rust and Python logs can be diffed in shadow mode:
  - base: `elapsed_seconds` (rounded to ms, from start), `source_port`, `length`, `handler_elapsed_ms` (≥ 25 ms only), `handler_state` (`not_ready`), `parse_error`
  - inspection: `ares_inspection`, `ciphertext_length`, `counter`, `client_packet_sequence`, `stateless_header`, `payload_bits`, `control_prefix_hex`, plus the `bunch_*`/`channel_*`/`has_*`/`header_flag_4`/`hardcoded_name`/`control_message_type` keys of §4.2 or `bunch_parse_error`
  - control chain: `sent_control_message_type`, `sent_ack_probe_length`, `sent_welcome_probe_length`, `sent_join_ack_length`, `sent_session_ack_length`, `acked_client_sequence`, `server_sequence`, `pre_control_sequence`, `pre_in_sequence`, `pre_state_error`
  - handshake: `packet_type`, `packet_count`, `network_version`, `client_id`, `sent_challenge_length`, `challenge_mode`, `matches_challenge`, `cookie_sha256_prefix`, `sent_ack_length`
  - inputs: `input_ack_received_event_id`, `input_ack_received_frame_length`, `input_ack_rejected_event_id`, `received_activation_events`
  - purchases: `received_purchases`, `purchase_resolved_path`, `purchase_unsupported`, `purchase_planned`, `purchase_decode_error`
  - settings: `received_range_settings`, `sent_client_update_range_settings`, `sent_infinite_ammo`, `range_settings_decode_error`
  - capture: `decoded_packet_hex`
  
  Never log keys or full plaintext outside the capture keys. Python logs only the 12-byte prefix plus the gated captures. Python also writes all retained events (deque of 8192) to the `--output` JSON file at exit: `{"bind", "total_events", "retained_events", "events"}` (`:4581-4584`).
- **Decode once.** Python decodes the same datagram up to 6 times: inspect, input, purchases, settings, capture, and the control-chain branch. Rust should decode once and hand the result to each consumer. Each consumer's gating and exception scope must stay independent: a purchase decode error must not suppress the settings decode.
- **RPM dependencies.** The handler keys, `read_pending_input_bounds`, `read_saved_moves` and `inspect_packet_notify` move to the `internal` oracle. Until then the decoder needs them as inputs. They are not derivable from packets.

---

## 12. Open questions (not resolved by Python; do not guess)

1. **64-bit prefix of client datagrams.** Its meaning is unknown. Python ignores it. Is it ever all-zero with length ≥ 57, so that `parse_handshake` would be attempted on a session packet? None was observed.
2. **Bit 75.** Python consumes it even when bit 64 (has packet info) is 0. The real 13.06 reader may read it only when bit 64 is set (as UE5's bHasServerFrameTime is). That has not been verified in IDA. The two are **not** equivalent in practice: 281 unique logged packets have bit 64 = 0, and Python misparses them by one bit (§3.1). Parity means reproducing the misparse. Should Rust fix it (a deliberate divergence)?
3. **`H & 0xF` (history word count).** Python assumes a single 32-bit history word. The client **does** send more: 1399 unique logged packets have counts 1-3 (§3.1). Python then reads a history word as the bunch header and decodes nothing from those packets. Parity means reproducing this. The fix is to skip `32 * (H & 0xF)` extra bits before bit 64. That needs IDA confirmation of the 13.06 header writer, and it is a deliberate divergence.
4. **Unhandled header cases.** The meaning of `header_flag_4` and of the 2 partial bits, the close-reason bits after bClose (the server writes `SerializeInt(reason, 15)`; the client reader stops), and non-hardcoded channel names (FString form) are not handled by Python.
5. **Unknown fields.** Controller fields 70, 83 (unreliable), 92 and 154, and pawn field 51 sent by the client, have no recorded identity.
6. **Dedup after wrap.** The dedup sets compare raw 12-bit sequences and collide after 4096 reliable ch5 bunches. Reproduce that exactly, or fix it (a deliberate divergence)?
7. **Uncaught join error.** A crash-causing `ValueError` occurs when a non-ch0 first bunch with body byte 9 arrives in the Join window (§9.5). Proposed Rust behavior: no join ack and keep waiting.
8. **Repeat purchases of Classic.** A purchase whose outer is a GUID the purchase path never exported (for example the starting Classic package) resolves to None, which logs `purchase_unsupported`. The 22 captured `<guid:N>` purchases name only purchase-allocated (odd) package GUIDs, and none is `BasePistol_C`. No capture shows whether the client does this.

---

## 13. Test vectors

All of these were generated offline by importing the Python modules. "Synthetic" means built with Python's own `BitWriter` / `encode_short_actor_rep_payload` / `transform_short_packet` and checked by Python's decoders. Every other vector is a real decoded client packet from `local_backend/runtime/*.out|*.log|*.json` (field `decoded_packet_hex` plus `payload_bits`; 16923 unique packets). The verifier re-generated every vector in this section independently and all match. The one exception is noted in §13.5.

### 13.1 Primitives

```
SerializeInt(v, max) -> width, value bits (LSB-first)
 156/179 -> 8 (0x9c)   157/179 -> 8   161/179 -> 8   170/179 -> 8 (0xaa)
 172..177/179 -> 8     176/179 -> 8 (0xb0)  177/179 -> 8 (0xb1)
 70/179 -> 7 (0x46)    83/179 -> 7    92/179 -> 7    154/179 -> 8 (0x9a)
 0/179 -> 8            1/179 -> 8
 pawn (max 54 = class max 53 + 1):
 51/54 -> 6 (0x33)     25/54 -> 5 (0x19)   41/54 -> 6 (0x29)   21/54 -> 6 (0x15)
 decoder, bits 1,0,1,0,1,1: max 54 -> 53 (6 bits) ; max 53 -> 21 (5 bits)   <- the max matters
packed uint (hex, bits)
 0:00(8) 1:02(8) 63:7e(8) 64:80(8) 127:fe(8) 128:0102(16) 255:ff02(16)
 525:1b08(16) 863:bf0c(16) 16383:fffe(16) 4294967295:ffffffff1e(40)
 ffffffff1e -> 4294967295 ; fffffffffe -> 34359738367 (35-bit, unmasked)
 ffffffff1f -> ValueError ; ffffffffff00 -> ValueError ; 01 (limit 8) -> ValueError("packed integer truncated")
FString
 0400000041626300 (64 bits) -> "Abc"
 fdffffff480069000000 (80 bits) -> "Hi"   (len 0xFFFFFFFD = -3, UTF-16)
 00000000 -> ""
```

### 13.2 Outer datagram (synthetic; key = bytes 00..1f, prefix = 0)

```
A) 77-bit ack-only packet, counter 0x6C3E3BFC, stateless 24
 datagram (28 B): 000000000000000018ff8e0f9b7566d7f672617f448a27d4c8fd650e
 -> decoded d036676500000000059a, P=77, counter 0x6c3e3bfc, stateless_header 24
B) Warden purchase, counter 0x12345678, stateless 12
 datagram (129 B):
 00000000000000000c9e158d44cd3889f9ff73a199862cc0541a4f91d73732ab6cc3e0313f85a881763fd110979137bd03e34deb43234b9cb5dd009326c0ae866869ade316e28b6b3f75d6f659477c7ec90b3fd9e86a56a7ade6fb8221b0007c1320728ae198bf776034d6b4b461c090b342710e31d80543df48ba6a244b6dbe03
 -> decoded == fixture bytes exactly (13.4 warden), P=883
 inspect_client_ares_packet -> {"ares_inspection":"authenticated","ciphertext_length":111,"counter":305419896,
  "client_packet_sequence":7346,"stateless_header":12,"payload_bits":883,"control_prefix_hex":"70e0ca72ffffffffc34c05f8",
  "bunch_start_bit":76,"bunch_header_end_bit":125,"bunch_is_control":false,"bunch_is_open":false,
  "bunch_replication_paused":false,"bunch_reliable":true,"channel_index":5,"has_exports":false,"has_must_map":false,
  "bunch_partial":false,"header_flag_4":false,"channel_sequence":863,"channel_sequence_start_bit":91,
  "hardcoded_name":true,"channel_name_index":102,"bunch_bit_count":757,"bunch_body_start_bit":125,
  "bunch_end_bit":882,"control_message_type":30}
Flipping one bit of byte 20 in either datagram -> None (tag mismatch).
C) P = 8200, validly tagged (1043 B, counter 5, stateless 3; plaintext bytes (i*37+19)&0xFF, trailer 0xCC)
 -> Python raises ValueError("unsupported packet bit count") out of decode_client_ares_packet. Rust: drop (§2.1).
```
Encoder used, i.e. what the client does:

```
T = transform_short_packet(plain, P, counter)
if P%8: T[-1] = (T[-1] & ((1<<tb)-1)) | ((0xCC << tb) & 0xFF); suffix = 0xCC >> (8-tb) (tb bits)
else:   suffix = 0xCC (8 bits)
(ct, tag16) = AES-256-GCM(key, nonce = le32(counter)‖le32(0x31ED27AB), T)
bits: prefix64 | sh(6) | counter(32) | tag16[0..4](32) | ct | suffix | 1
```

Inverse transform alone (`inverse_short_packet`). The input is `bytes((i*37+19) & 0xFF)` of length `ceil(bits/8)`:

```
bits=77  ctr=0x0        in 13385d82a7ccf1163b60 -> 51aca7c3dc347dbe497f
bits=77  ctr=0x6c3e3bfc in 13385d82a7ccf1163b60 -> 77205dcf6a347dbe0b77
bits=100 ctr=0x1        in 13385d82a7ccf1163b6085aacf -> 51aca7cdf3347dbe8526bdbcca
bits=232 ctr=0x12345678 in 13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f
                        -> 7ea2ecd26b347dbe0e882691c115eaa3c71850fb70c4ad11b9a56e8771
bits=883 ctr=0xdeadbeef in 13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f44698eb3d8fd22476c91b6db00254a6f94b9de03284d7297bce1062b50759abfe4092e53789dc2e70c31567ba0c5ea0f34597ea3c8ed12375c81a6cbf0153a5f84a9cef3183d6287acd1f61b40658aafd4f9
                        -> 97db7aaaf3347dbe3bb11a09c215eaa32fbf4df56ec4ad1134cd06dfd992a90212b587842beb5dd455581ee9900b44cfc788bd1df50e85fae99d8db5dc4b8831e0c07659bddec7b3b0e97def65ef1b5535afe47826c93256659c577d6b7fd393dd9d0f5e5e21b6034ffb5c22e4d6fb
```
Also run `test_native_transform_roundtrip` (`test_range_udp_handshake.py:200-204`).

### 13.3 Control bunches (synthetic, from `test_range_udp_handshake.py:260-330`)

```
Hello (P=232): 0000000000000000ff3701a02cfe0b8001000000000000000000000080
 inspect_control_bunch -> {"bunch_start_bit":76,"bunch_header_end_bit":135,"bunch_is_control":true,"bunch_is_open":true,
  "bunch_replication_paused":false,"bunch_reliable":true,"channel_index":0,"has_exports":false,"has_must_map":false,
  "bunch_partial":false,"header_flag_4":false,"channel_sequence":357,"channel_sequence_start_bit":93,
  "hardcoded_name":true,"channel_name_index":255,"bunch_bit_count":96,"bunch_body_start_bit":135,
  "bunch_end_bit":231,"control_message_type":0}
Login type 5 (P=232): 0000000000000000ff3701a02cfe0b8081020000000000000000000080 -> control_message_type 5
Join  type 9 (P=232): 0000000000000000ff3701a02cfe0b8081040000000000000000000080 -> control_message_type 9
Ack-only (P=77): d036676500000000059a
 bits 64..76 = 1,0,1,0,0,0,0,0,0,1,0,1,1 ; inspect_control_bunch -> {} (P < 128)
 H = 0x656736d0: seq = (H>>18)&0x3FFF = 6489, client's acked server seq = (H>>4)&0x3FFF = 13165
   -> Challenge server_sequence = 13166, next_server_sequences = 13167, initial_actor_channel_sequences = 13167 & 0xFFF = 879
```

### 13.4 Real decoded client packets: bunch walk

The format is `name: P bits` and the hex, if under 2000 bits, followed by each `iter_bunches` entry: `ch, rel, seq, start, body, end` and the content block `(rep, offset, bits)`, field and params size.

```
warden (fixtures/server_purchase_warden_packet.json): P=883
 70e0ca72ffffffffc34c05f89accf5c263853ff6b0a2f1303e443490f6b7c1cf6f62d82c528570892e7ea0c3eeaa850a6b3cad92f9a426ae671d6eb19d7127cd9320a2e5f51b320f728552a5220f16f6fd8c03a4d5757d5c3c049a3ee8c81b35e0231e62d9d4dbbe9b0bc73bb23c64
 header 0x72cae070: seq 7346, acked 11783, low4 0, history 0xffffffff, bit64 1, packet-info 609, bit75 1
 [ch5 rel seq=863 start=76 body=125 end=882] block(rep=0, off=143, bits=739) field 156, params 715
acr (responder_28576 @379.282): P=1328
 0080e258ffffffff794a0538a8cc25c313063ef6b0cae6313ec4321e485e67e92613b8050c7ff4915318c544dfd2cb53af029823eb09e63478aeb1c1d5db01b63f8ebc95a49363ff2fbe1741e3c02e2014efc53ce98234127cc40603dfb7dc22b777950bf0baa0e64959c01259c79dd746c2ebe20002e282bf046061df34cbd32aadb5e5e3d911a489c5e4d11614c6e625f38e2ab69035aaac3a079e05f44b5c033a51dd16c3
 [ch5 rel seq=1287 76/125/930] block(0,143,787) field 156, params 763
 [ch8 unrel 930/958/1327] block(0,976,351) field(54) 51, params 329
settings (responder_24252 @148.422): P=1464
 f0b43dcfffffffff874a0570eeccafc01d81b1b030280ba02a3fd7f8baec1b229ad93ec67d4b0578eecc1d4013c6bb010242881f106561df6413d87c30e6454aa3630de6a8e2e2d0ac1c866f7f5a232cc79c8614acea4b98261d0e03c4d7367e587453a642735777a80d4ab2f08a5c08d0ceff8f26860476aae00d753452050bfe158c60df438986d48069b6ea321b8cb4f97cebc6f9091cde355a5ef418a9eb444e06ead2731ffe2134856d403236b7b641ed7ce43991
 [ch5 rel seq=3534 76/125/300] block(0,143,157) field 176, params 133, payload b00b020900000000000000000000002000000000
 [ch5 rel seq=3535 300/349/378] block(0,359,19) field 177, params 3, payload b10604
 [ch8 unrel 378/406/1463] block(0,424,1039) field(54) 51, params 1017
 first-bunch control_message_type = 238 (meaningless off ch0)
ch5_field92 (responder_24252 @146.485): P=1826
 90b155ceffffffff0d4b0560eecc67c25584349f60980cea0e17a96d6ef08b52ebb7c3d6b3d2acc3c1eee7df93feb38edde4caec0c13e35438eb6a4529c3d7caa5d5db5da94b2da092bac6803f2a52ed45a09cd4f013c009a6195888070808217e4094857d934d60f3c19817298d8e3598a38a8b43b3b61bbead6a8db01c731a52b0aa2f619a74380c105fdb7c62d14d990acd5ddda13628c9c22b7221403bff3f9a1812d8a98237d4d148452ef85730827d0f251a5203a6d9aacb6c30d2e6f3ad1be7277078d76879d163a4ae139d18a823ce7df887d014b601c9d8dcda06b5f391e74432
 [ch5 rel seq=3532 76/125/740] block(0,143,597) field 92, params 574 -> purchase: none, settings: none
 [ch8 unrel 740/768/1825] field(54) 51
ch5_field70 (range_launch_23872 @100.859): P=1573
 e0c40efbffffffff514b0548efcc224098c7b11840405af195822cec9ba2649f72ca196321d6dd11ff2722b4b0d6e292aeb3ac7ff1abe2b2d1be9d6916293e28600f2a8739b9da6efde63b3f80710dcd7db87f7b526ac4b1dcd01671ab25465ae5b22fa6ae3d51fff78d0bd363c9680dab28312068e766666443edbc08618eedd9230372c3748812dd4462f59b076b6d37b263408dba0b9473c17c1d78c0b33482a43c1768d076462d1e58d45bf82ba3140fbd0e50d33c4eca25ead06bd423cf649d5ba692
 [ch5 rel seq=3561 76/125/159] block(0,135,24) field 70 (7 bits), params 9, payload 468914
 [ch8 unrel 159/187/1572] field(54) 51
ch5_field154 (responder_13096 @621.218): P=220
 309bcaa4ffffffff194d05c8a9cc1f401557445d100072eabf0000c8
 [ch5 rel seq=1337 76/125/156] block(0,135,21) field 154, params 5, payload 9a0a05
ch5_unreliable_field83 (range_launch_23872 @97.297): P=343
 30c4dafaffffffffa90e057007e60a8c85151bef2f84e9ff71a02c4cd785ef0b1bec67b872d05ad5425766
 [ch5 UNREL 76/104/342] block(0,122,220) field 83, params 197 (decoders ignore: not reliable)
ch5_subobject (responder_31280 @920.781): P=1050
 f07587d7ffffffff274c0540d3cc0f028102040d0000002a34b9373a3ab632b2a63038a834b7b3a1b736b837b732373a001f7755e55501dcb47a6b442de03afdced4c9f272c2d71d2e150d21221ec4ae2c0908880bfe1280857dd32c4fabb4d61b8e174490e6285fbd0d3cb79b2b95214d9b2b9f0d35911e782a6413d70ce83432e30633
 [ch5 rel seq=2664 76/125/652] -> ValueError("subobject block needs its own NetGUID context"); bunch skipped
 [ch8 unrel 652/680/1049] block(0,698,351) field(54) 51
ch8_plus_ch4 (range_capture_25768 @56.125): P=6158 (hex omitted, see log)
 [ch8 unrel 76/104/4049] block(rep=1, off=122, bits=1963) -> decode_received_actor_inputs = ()
 [ch4 unrel 4049/4077/5117] -> content length exceeds bunch
ch8_overrun (responder_31504 @140.094): P=4145
 [ch8 unrel 76/104/4145] end == P accepted by iter_bunches; content block -> ValueError("actor content length exceeds bunch")
ch8_subobject (responder_13096 @111.734): P=5738
 [ch8 rel seq=3346 76/125/188] -> subobject ValueError ; [ch8 unrel 188/216/5737] field(54) 51
ch5_partial (responder_30456 @78.157 and @90.515): P=7656
 [ch5 rel PARTIAL seq=445 / 958, 76/127/7383] -> both decoders return None (partial)
```
`decode_purchase_requests(bytes(64), 400) == []`.

Header misparse vectors. These are Python's actual behavior, which Rust must reproduce for parity (§3.1). Only `control_prefix_hex` is logged for these packets, so the expected values are the logged inspection keys:
```
history-count 1 (responder_24252 @392.875, P=506): prefix e1c19270ffffffffffffffff
 H=0x7092c1e1: low4 = 1, seq 7204 ; Python -> {"bunch_start_bit": 76, "bunch_is_close": true}, iter_bunches -> []
bit64 = 0 (responder_24252 @76.204, P=1567): prefix c079ddb7ffffff01a840c96d
 Python -> bunch_start_bit 66, body 104, channel_index 4738, bunch_partial true, has_must_map true,
           header_flag_4 true, replication_paused true, unreliable, bunch_bit_count 3277, bunch_end_bit 3381 (> P),
           control_message_type 80
 (with bit 75 skipped: bunch at 65 -> ch5, reliable, partial, ChSequence 3529)
```

### 13.5 Purchases

```
warden -> [PurchaseRequest(field=156, item_path='/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C', target_slot=0, purchasable_type=0, channel_sequence=863)]
 content wire (raw, 739 bits): 7fec6145e3617c886820ed6f839fdfc4b059a40ae1125dfc4087dd550b15d6785a25f3494d5ccf3adc623be34e9a274144cbeb37641ee40aa54a451e2cecfb190748abebfab87808347dd091376ac0473cc4b2a9b77d37178e77647900
 decoded payload (actor_map 4): 9c970a050a040a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84c2e8e8d8caa4d2ccd8ca5e84c2e8e8d8caa4d2ccd8ca00000000001c00000084c2e8e8d8caa4d2ccd8cabe8600de0efd6e01
acr -> [PurchaseRequest(156, '/Game/Equippables/Guns/Rifles/Carbine/AssaultRifle_ACR.AssaultRifle_ACR_C', 0, 0, 1287)]
settings packet, ch5_field92 -> []
synthetic repeat purchase (P=382; seq 100, acked 50; ch5 rel seq 1290; no channel name; params:
 present 1, NetGUID 1, flags 0x01, outer NetGUID 525, FString "AssaultRifle_AK_C", slot present 1 + 0x00, type present 1 + 0):
 20039001ffffffff014005502808c17601d89f6590129a18bf5b69464768a82caf2bab3edd4fcd9cb385c0b7370ee83e
 -> [PurchaseRequest(156, '<guid:525>.AssaultRifle_AK_C', 0, 0, 1290)]
 resolve_class_path(..., {525: '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK'}, {})
 -> '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C'
synthetic sell (P=155; ch5 rel seq 7; field 161; item absent, slot present 11, type absent):
 20039001ffffffff014005380025401b5e685e05 -> [PurchaseRequest(161, None, 11, 0, 7)]
REAL repeat purchase, <guid:N> form, flags 0x05 + checksum (responder_20928 @2852.921), P=411:
 a000438effffffff3b490598decc1dc10b8259f6e5ea9f123ebf54c17404bbca7b7564a981ca76849b77044b92c213c427d65f65
 [ch5 rel seq=3027 76/125/410] block(0,143,267) field 156 params 243
 decoded payload 9ce702050a36102400000082e6e6c2ead8e8a4d2ccd8cabe8296be8600de0efd6e01
 -> [PurchaseRequest(156, '<guid:525>.AssaultRifle_AK_C', 0, 0, 3027)]
REAL repeat sidearm, TargetSlot present = 1 (responder_20928 @181.734), P=411:
 70e7532cffffffffdd4d0528cbcc1dc10b020bf6cc9b9f123e3fafc61661bbca7ef5d1b97d60544bfe1c9dd7d92d47ab5963e265
 decoded payload 9ce702050af61822000000a4caecded8eccae4a0d2e6e8ded8be8600de0efd6e0700
 -> [PurchaseRequest(156, '<guid:829>.RevolverPistol_C', 1, 0, 2405)]
REAL repeat, short class name (responder_20928 @2817.031), P=339:
 f047a246ffffffff154a0578deccd5c043811af6e374bc063e3fe016939209cb3684daf3d487fe75c1bf65
 [ch5 rel seq=3023 76/125/338] block(0,143,195) params 171 -> [PurchaseRequest(156, '<guid:683>.DS_Gun_C', 0, 0, 3023)]
REAL armor purchase (responder_20928 @1293.078), P=1088, bunches [ch5 rel 792 76/125/690][ch8 unrel 690/718/1087]:
 f0487729ffffffff114905c098cc35c223843ef6b002f700beb4d09b1f2da945cda9336c0fb82b962afa765d7a3f70fe976b3db468499abf0ffc9eab169ad1dd6ccc26236c3f8daa347c9e8bf96bc80c500d5b588df40102e282bf046061df34cbd32aadf586e30511a439ca9b6f03166ee6d2ae4cd34dee674b4da4079e0ad987df033a8dccb8c1
 -> [PurchaseRequest(156, '/Game/Gear/HeavyArmorItem.HeavyArmorItem_C', 11, 0, 792)]
```
Note: the synthetic bunches have `bHardcodedName = 0` and **bit 75 = 0** (byte 9 = `0x40`; the real client always sends bit 75 = 1). Python reads no name (§4.1) and ignores bit 75, so they decode correctly, but they are not byte-identical to client traffic. Real client bunches carry name 102. The synthetic 0x01-flag (no checksum) form decodes the same way as the real 0x05 form.

### 13.6 Settings and content transform

```
settings capture -> RangeSettingsRequest(field=176, channel_sequence=3534, infinite_ammo=True, bot_amount_index=1, rest 0/False)
                    RangeSettingsRequest(field=177, channel_sequence=3535, is_init_settings_flow=True, rest 0/False)
synthetic, 3 bunches in one packet (P=482): seq 4094 field 174 (no params); seq 4095 field 177
 (distance present = -2, bot_enabled 1, init 0); seq 0 field 172 (object present, NetGUID 1, flags 0x01, outer 0, "/Game/X/Mod")
 20039001ffffffff014005f07f1a40906d582ac0ffeb019a21691fcf63c6920a000086816302633f618fc509d47ee2369f8080b2fbd378420a98812d03
 bunches (ch, seq, start, body, end): (5,4094,76,117,143) (5,4095,143,184,245) (5,0,245,286,481)
 -> 174 defaults ; 177 distance_index=-2 bot_enabled=True ; 172 object_path='/Game/X/Mod'
decode_content_payload(wire, bits, actor_map) -> plain
 bits=9   map=4  wire 0800                                 -> 1300
 bits=31  map=4  wire bf63ff4f                             -> 13385d02
 bits=32  map=4  wire 5a2a2cd0                             -> 13385d82
 bits=33  map=10 wire 1a212c9300                           -> 13385d8201
 bits=64  map=10 wire eb565e03d857b905                     -> 13385d82a7ccf116
 bits=65  map=4  wire eb565e2ed89a5f0501                   -> 13385d82a7ccf11601
 bits=97  map=10 wire eb565e82d82775052ce71ecf00           -> 13385d82a7ccf1163b6085aa01
 bits=135 map=4  wire e9565e62d8fa75059de91279eaf20d1040   -> 13385d82a7ccf1163b6085aacff4193e63
```

### 13.7 Input streams (ch8, actor map 10)

```
A) test_first_captured_channel8_packet: decode over bits [104, 553)
 200a2183ffffffff470f08080e7e1988917d93132198b5ea5aaea301ca0ec955a30fd194da748ad20fe660abeeaf6430c159400b58796b0668f7ed82b6d5e8838e9a0ab60733
 block offset 122, bits 431, field(54) 51, params 409
 decoded content f38c4117000000a9000008000004000400fc14b042dc15f6a8ad5e7f200000000000000000400b180d00000040d00000000012005200
 -> 1 stream: marker 315, end 415, ids 0..1, kinds (6, 4), event[1].object_guid 0
B) test_captured_slot_two_event: bits [104, 609)
 c034319cffffffff9f0908c80f3e1b88857d13aa6f45a06ca876d21ec60eea4dd5b7a234a902a3512f65d00a416202b33288d830a45bebdb8d12e628f6a9b5d12b0ac07e681c8c75e1c5830933
 block 122/487, field 51, params 465
 -> 1 stream: marker 315, end 471, ids 370..372, kinds (7, 1, 5); event 371 slot 2 flag 0 state 0 action 0
C) range_launch_21904 @182.891, P=578, bits [104, 577)
 80b4fe64ffffffff890d08c80e3e1a88857d13b47510a0c0be321ce4f6544a664f71398cc0e029e348ba18c6fa04af2d80b49f66da98960eae8b1d488ebd4b38be8113b6419b12e832
 -> 1 stream: marker 315, end 439, ids 10450..10452, kinds (7, 1, 4); event 10451 slot 2 flag 1 state 0 action 0; event 10452 object_guid 204
D) multi-stream (responder_28576 @126.563), P=2570, first bunch ch8 unrel bits [104, 2569), block 122/2447
 50e35361ffffffff990c08084d7e9808857d93fdd246258d3e0f17f191d4256142171abc080d7977e37546691a7e925f8246a55f5a3be061a835c32791241266572b97c4bb97be04a4642050f4e5f88bdce2451d52f15ec89e67e812c0a79b4684a1b31e2d6c46d2ca3d33e988ef568b7f9209a0e5bce08f0ba4d84f7cc238d3db90dd334970788ab79a184fb8673de9642eb9ece820db91e27bf81d1e90b1ebe85fa285490b2702fdc818782c3650a6f706f8a097b222d1571554fdfa1e7bfbf247037d7e5e1540c903feab71de79b5b2089b4483ed84317b3681cb59f9930bc61848c58b76dd1c14c59194750c9a9f13657e3ddb37fda553d2ab0c276f6cce34708d3874b7a88f6758021191f1541ce1c1a512822d5bbac53901dbcc918adee5bfde032ae6b9bdf2b5614310d5bdbe0bba91d1a3857c88d39faa342aa5a79ef333
 -> 4 streams (Python sends NO input ack for this packet):
    marker 315 end 2426 ids 7139..7179 (41 events, kinds 7/5 only)
    marker 497 end 568  id 3809742614 kind 8 guid 2024     (spurious, inside the first)
    marker 693 end 764  id 3658753302 kind 8 guid 9704     (spurious)
    marker 746 end 817  id 3579064470 kind 8 guid 1512     (spurious)
E) synthetic (test_synthetic_received_activation): 13 zero bits, A3, 1, id 42, count 2, [1,kind1,(1<<8)|(2<<5)|(2<<1)|1],
   [1,kind7,0xA5A55A5A], 0, 16 zero bits; content encoded with actor map 10; body = 0,1,packed(len),content
   plain content 0060b40a000000c128cad3d22a2d050000 (132 bits); body 26088c85a134653f372e863b726d8d07139418 (150 bits)
 -> 1 stream: marker 13, end 116, ids 42..43; event0 kind 1 slot 2 flag 1 state 1 action 2; event1 kind 7
    max_id 43 -> same; max_id 41 -> ()
F) rejects, scan_received_input_streams(plain, bits) == () (test_range_client_input.py:96-120):
   a300 (9)                       bit after marker = 0
   a3010000000400 (49)            count 1 (packed 0x02) but no event bits (truncated)  <- NOT a count-0 case
   a30f000000043e (55)            count 1, event kind 15 (unknown)
   a3c900000004460100 (65)        one kind-1 event, no terminal bit
   a3c9000000000000 (57)          count 0 (packed 0x00) -> rejected by 1 <= count  (verifier-added)
   accepted minimum (verifier-added): 3 zero bits, A3, 1, id 7, count 1, [1, kind 2, 1 payload bit], terminal 0
   187d000000205002 (59)          -> 1 stream: marker 3, end 59, id 7, kind 2
Kind counts in first-bunch-ch8 packets with exactly one stream (16923 captures): 0:934 1:996 3:86 4:447 5:3545
 6:454 7:962 9:103 10:58 11:1 (the kind-11 is the spurious stream of §8.4). Kind 8 appears only in the
 spurious extra streams of multi-stream packets (those are single kind-8 events in 4 cases, kind-4 in 1).
 Kinds 2 and 12 never seen.
```

### 13.8 Ack history and class-path resolution

```
update_received_packet_history: fresh deque: 100 -> 0x0, 101 -> 0x1, 104 -> 0xc, 136 -> 0x80000000
 deque [0x3FFF] then 0 -> 0x1 ; fresh: 5 -> 0, 5 -> 0, 6 -> 0x1 (duplicate 5 appended twice) ; 0x4000 -> ValueError
resolve_class_path (packages = {525: '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK'}, AK = that + '.AssaultRifle_AK_C')
 '<guid:525>.AssaultRifle_AK_C', packages, {}        -> AK
 AK, {}, {}                                          -> AK (pass-through)
 '<guid:9>.AssaultRifle_AK_C', {}, {'AssaultRifle_AK_C': AK} -> AK
 '<guid:9>.AssaultRifle_AK_C', {}, {}                -> None
 '<guid:9>', packages, {}                            -> None
 None / '' / '<guid:x>.A_C' / '<guid:1>'             -> None
 '<guid:525>.Foo.Bar_C', packages, {}                -> '/Game/.../AssaultRifle_AK.Foo.Bar_C' (then split_class_path
                                                        rejects it: 2 dots -> store_item None -> purchase_unsupported)
 'Name', {}, {}                                      -> 'Name' (pass-through; split_class_path then rejects it)
 Python int() quirk: '<guid:+525>.A_C' resolves like 525 ('+', whitespace and '_' are accepted by int()).
 Decoder-produced strings are always canonical decimal, so Rust may parse plain digits.
split_class_path:
 '/Game/A/B.B_C' -> ('/Game/A/B', 'B_C', 'Default__B_C') (store lookup None) ; '/Game/A/B.C_C' -> ValueError
 '/Game/Gear/HeavyArmorItem.HeavyArmorItem_C' -> HeavyArmorItem_C (Heavy Shields, slot 11, not a gun)
```

### 13.9 Reply bodies triggered by decoded input (verifier-added)

These are the bunch bodies (before framing) that the §7.4 and §8.5 reactions send. They were generated with Python's builders; framing is in the server-builder doc.
```
170 request: build_client_update_range_settings_payload() -> aa00 (16 bits)
             build_controller_rpc_body(...)                -> 8290c002 (26 bits)   send_reliable(5, ..., has_exports=False)
infinite ammo: build_game_state_bool_body(46, True)        -> 9350d10f (28 bits)   send_reliable(4, ..., False)
               build_game_state_bool_body(46, False)       -> 9350f10d (28 bits)
input ack (last_event_id 372): build_input_ack_rpc_payload(372, 0, 41, 53) -> 2961ba00008000000000 (80 bits)
             encode_short_actor_rep_payload(..., 0, 10) + append_actor_rpc_content_block -> 825e85856554a5e984b30e00 (90 bits)
             sent unreliable on ch8 at next_server_sequences[source]
```

### 13.10 Existing Python tests to port as Rust tests

- `test_range_client_rpc.py` (Warden fixture walk and decode, garbage)
- `test_range_client_input.py` (content roundtrip, the 2 captured streams plus 1 synthetic, rejects)
- `test_range_udp_handshake.py:20-29` (ack history), `:200-204` (transform roundtrip), `:260-338` (control bunch framing)
- `test_range_gun_loader.py:27-39` (`resolve_class_path`), `:41-53` (`split_class_path`)

All four files pass offline (verifier run, 2026-10-14: 9 + 21 + 9 tests OK).

---

## Verifier corrections

An adversarial re-check against the Python source, with every vector re-generated offline (scratchpad `verify06/`). It covered the 16923 unique captured packets and all 4.39 M unique logged `control_prefix_hex` values. Changes made in this file:

1. **§3.1, §12 Q2/Q3, §13.4.** The claim that `H & 0xF == 0` and bit 64 = 1 always hold was false. 1399 logged packets have history count 1-3, and Python misparses them as a close bunch at bit 76, decoding nothing (reliable RPCs inside are lost). 281 have bit 64 = 0, and Python misparses them by one bit. Added the stats, the consequences and vectors. Parity requires the fixed-offset parse.
2. **§6.2, §6.1, §12 Q8, §13.5.** The repeat-purchase `<guid:N>.Class_C` form **is** captured: 22 bunches. The real client sends flags **0x05** with checksum `0xb77e876f`, not 0x01. Added the real vectors (AK `<guid:525>`, Sheriff with TargetSlot 1, Outlaw, Heavy Shields with slot 11). TargetSlot is present for sidearms and armor in real traffic.
3. **§1.** "Every send carries `ack_history_mask` and increments the sequence" was wrong. The Challenge sends history 3 with sequence client-acked+1 and **sets** `next_server_sequences`. The Welcome sends history 7, the join ack history 3. Added a per-sender table.
4. **§9.3.** Added the missing `& 0x3FFF` on the Challenge `server_sequence` and on `next_server_sequences`.
5. **§9.5.** The join ack also sets `last_session_ack_times`, so the first session ack waits ≥ 0.1 s. This is confirmed in the logs.
6. **§9.2.** The Hello is retransmitted by the client. A non-Hello 232-bit packet before the Welcome corrupts the Welcome template and crashes Python (uncaught `ValueError`). The 232-bit branch also skips the setup chain and the move ack nested in the session-ack branch.
7. **§2.1, §13.2 C.** A validly tagged `P > 8192` datagram raises an uncaught `ValueError` from `inverse_short_packet` and kills the responder. It was missing from the reject list.
8. **§5.1, §5.4, §8.1, §13.1, §13.4.** The pawn RPC field is `SerializeInt(·, 54)` (class max 53 + 1), not 53. The widths coincide for 51/41/25 but not in general (21: 5 vs 6 bits).
9. **§13.7 F.** The reject cases were mislabelled: the "count 0" case is really count 1 with no events, and the continuation-bit-0 case was missing. Added the hex of every reject, a real count-0 case and a minimal accepted stream.
10. **§8.5.**
    - Unreliable ch8 bunches carry **no** name or sequence on the wire; the spec said "name index 102".
    - The gate does not check partial or reliable.
    - The bounds are by byte length.
    - Added the RPM pointer chain and the `count > 4096` reject.
11. **§5.2.** Completed the `ValueError` list of `read_actor_content_block` (bounds, truncated header, `bit_count == 0`).
12. **§6.3.** `iter_bunches` can raise (`P < 76`, pre-loop reads); the spec said it could not.
13. **§4.3, §5.2, §5.4, §8.1, §8.4.** Refreshed the capture statistics from 7212 to 16923 packets:
    - more ch5 field sizes (154/149, 83/229, many 156 sizes)
    - 3 partial bunches
    - subobject counts 4 (ch5) and 5 (ch8)
    - stream-count distribution
    - a spurious **single** stream (kind 11) that only the RPM bound rejects
14. **§7.4.** `received_range_settings` also drops a falsy `channel_sequence` (0). Documented the `range_hud_sent_times.get(source, now)` default and the `repr(error)[:200]` formats.
15. **§11.** Completed the log key list (base, control chain, handshake, `handler_elapsed_ms`, startup lines, exit JSON file). Fixed "decodes up to 4 times" to 6. Added that RPM must not block the 128 tps tick, that clocks are read per datagram, and that idle ticks send nothing.
16. **§1, §9.3.** Documented the blocking RPM: a 30 s key scan, and `inspect_packet_notify` taking 4797 ms in a real run. Keys are acquired before `parse_handshake`, so handshake datagrams are dropped until then.
17. **§0.1.** Added the dead launcher and env switches: `--pawn-authority-role`, `VALORANT_RANGE_DROP_FLAGS`, `RANGE_ACR_ACTOR_INSPECTION_PROBE`, `RANGE_EQUIP_STATE_TRIAL`, `RANGE_BASE_PISTOL_SKIN_TRIAL`.
18. **§13.5.** The synthetic vectors have bit 75 = 0, unlike real traffic. Noted.
19. **§10.** Added `ack_sent_sources`, `welcome_sent_sources`, `join_ack_sent_sources`, `last_session_ack_times` and `purchase_jobs`, and corrected the `next_server_sequences` init.
20. **§13.8 (resolve-path quirks) and new §13.9 (reply bodies: 170 request, bool 46, input ack).** The old 13.9 is now 13.10.

Verified unchanged and correct:
- the outer GCM layout and the A/B datagram vectors (byte-exact)
- the inverse transform and content-transform vectors
- Warden, ACR, settings and the other captured walks
- every synthetic purchase/settings vector
- the ack-history vectors and the store table (all 23 rows)
- the authority-block order and the sbox-inverse equality
