"""Compare a python responder log with an aries-server events log.

usage: python tools/shadow_diff.py <python responder .out or .json> <logs/aries_events_<pid>.jsonl>

Both write one JSON object per received datagram with the same key names. Timing
(packet sequences, elapsed seconds, how many datagrams a client sent) differs run to
run, so this compares what each server *did*, in order:

- milestones: the first time each significant key appears (bring-up stages, hud,
  streaming, purchases, bots...), in order, with the value it carried
- counts: how many events carried each significant key
- values: for keys whose values are deterministic (bring-up frame lengths, bodies'
  bit counts), every distinct value seen

Keys every packet has (python's quiet list, range_udp_handshake.py:1378-1393) and keys
only the rust server writes are left out.
"""

import json
import sys
from collections import Counter, OrderedDict

ROUTINE = {
    "elapsed_seconds", "source_port", "length", "ares_inspection", "ciphertext_length", "counter",
    "stateless_header", "payload_bits", "control_prefix_hex", "bunch_start_bit", "bunch_header_end_bit",
    "bunch_is_control", "bunch_is_open", "bunch_replication_paused", "bunch_reliable", "channel_index",
    "has_exports", "has_must_map", "bunch_partial", "header_flag_4", "channel_sequence",
    "channel_sequence_start_bit", "hardcoded_name", "channel_name_index", "bunch_bit_count",
    "bunch_body_start_bit", "bunch_end_bit", "control_message_type", "sent_session_ack_length",
    "acked_client_sequence", "server_sequence", "sent_move_ack_length", "move_ack_value_source",
    "move_ack_value", "move_ack_newest_number", "move_ack_timestamp", "move_ack_oldest",
    "move_ack_last_acked", "move_ack_count", "decoded_packet_hex", "client_packet_sequence",
    "handler_elapsed_ms", "parse_error", "bunch_parse_error", "bunch_is_close",
}

# written only by the rust server (retransmission, decided fallbacks, diagnostics)
RUST_ONLY = {
    "resent_reliable", "resend_error", "stage_error", "hud_rpc_error", "bot_shot_error", "bot_tick_error",
    "cloudburst_error", "purchase_error", "base_pistol_initial_cosmetic_error", "range_streaming_error",
    "range_hud_error", "range_settings_error",
}

# high rate, values vary with timing: counted, not listed
NOISY = {
    "sent_equippable_auth_states", "input_ack_received_event_id", "input_ack_received_frame_length",
    "input_ack_rejected_event_id", "received_activation_events", "movement_ack_path", "move_correction_location",
}


def load(path):
    events = []
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if isinstance(event, dict) and "elapsed_seconds" in event and "status" not in event:
                events.append(event)
    # a python --output file is one JSON list instead
    if not events:
        try:
            with open(path, encoding="utf-8") as handle:
                data = json.load(handle)
            events = [e for e in data if isinstance(e, dict)]
        except (ValueError, OSError):
            pass
    return events


def significant(event):
    return {k: v for k, v in event.items() if k not in ROUTINE and k not in RUST_ONLY}


def summarize(events):
    milestones = OrderedDict()
    counts = Counter()
    values = {}
    for event in events:
        for key, value in significant(event).items():
            counts[key] += 1
            if key not in milestones:
                milestones[key] = value
            if key not in NOISY and isinstance(value, (int, str, bool)) and key.endswith(("_length", "_bits")):
                values.setdefault(key, set()).add(value)
    return milestones, counts, values


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    python_events, rust_events = load(sys.argv[1]), load(sys.argv[2])
    print(f"python: {len(python_events)} events, rust: {len(rust_events)} events")
    p_mile, p_count, p_values = summarize(python_events)
    r_mile, r_count, r_values = summarize(rust_events)

    print("\n== milestone order (first appearance) ==")
    p_order = [k for k in p_mile if k not in NOISY]
    r_order = [k for k in r_mile if k not in NOISY]
    common = [k for k in p_order if k in r_mile]
    r_common = [k for k in r_order if k in p_mile]
    diverged = False
    for index, (p, r) in enumerate(zip(common, r_common)):
        if p != r:
            print(f"  first order difference at #{index}: python {p!r}, rust {r!r}")
            diverged = True
            break
    if not diverged:
        print(f"  same order for all {len(common)} shared milestones")
    only_python = [k for k in p_order if k not in r_mile]
    only_rust = [k for k in r_order if k not in p_mile]
    if only_python:
        print(f"  only in python: {only_python}")
    if only_rust:
        print(f"  only in rust: {only_rust}")

    print("\n== first values that differ ==")
    shown = 0
    for key in common:
        if key in NOISY or p_mile[key] == r_mile[key]:
            continue
        print(f"  {key}: python {json.dumps(p_mile[key])[:160]} | rust {json.dumps(r_mile[key])[:160]}")
        shown += 1
    if not shown:
        print("  none")

    print("\n== deterministic lengths and bit counts ==")
    for key in sorted(set(p_values) | set(r_values)):
        p, r = p_values.get(key, set()), r_values.get(key, set())
        mark = "  " if p == r else "!!"
        print(f"{mark} {key}: python {sorted(p)} rust {sorted(r)}")

    print("\n== counts ==")
    for key in sorted(set(p_count) | set(r_count)):
        mark = "  " if p_count[key] == r_count[key] else " ~"
        print(f"{mark} {key}: python {p_count[key]} rust {r_count[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
