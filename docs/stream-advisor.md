# Stream settings advisor

Settings → Quality → **Suggest settings**. The app sends the server what it
knows: this computer (platform, CPU cores, screen size, measured refresh rate,
decoders), the chosen host (platform, GPU, codecs), the network (last route
used, host latency, Network check UDP result and server round trip). The
server answers resolution, FPS, bitrate, codec and network mode plus 1–3
reasons. **Use these settings** saves them as the Custom profile.

A goal (balanced, smooth and stable, sharp and stable, fastest response)
leans the answer one way: more FPS, more resolution, or smaller frames with
H.264.

## Live measurement

**Suggest** first measures for a few seconds (`/api/v2/advisor/probes`, memory
only, 10 per account per 10 minutes):

1. The viewer opens a probe for one of its own online hosts.
2. The host, polling every 2 s while online, claims it, measures its round
   trip to the server and uploads 256 KiB chunks for 2.5 s, then reports its
   upload Mb/s plus CPU, memory, GPU, main screen and refresh rate (read with
   `sysctl`/`system_profiler` on macOS, CIM on Windows). A host in a session
   reports its machine only, so the session keeps its bandwidth.
3. The viewer measures its own round trip and downloads chunks for 2.5 s.

The relay runs on the server, so viewer→server plus server→host is the relay
path; a direct path is usually at least as good. The rules then budget 60% of
the slower of the two speeds (goals never go past 80%), and pick resolution
and FPS from that budget. Without an answer from the host (offline, older
Sanser or older server) the app says so and the latency rules answer.

## LM Studio behind the server

The language model is reached only by the Sanser server, never directly by
the apps, so it is not open to the internet. Set on the server (Render →
Environment):

| Variable | Example |
| --- | --- |
| `ADVISOR_LLM_URL` | `https://lm.example.com` (https; http only for localhost) |
| `ADVISOR_LLM_MODEL` | `qwen2.5-3b-instruct` (default) |
| `ADVISOR_LLM_API_KEY` | optional bearer token if the tunnel/proxy needs one |

LM Studio: load a small instruct model (Qwen2.5 1.5B/3B Instruct or Llama 3.2
3B Instruct are enough), Developer → Start server (port 1234), then publish it
over HTTPS with a tunnel (Cloudflare Tunnel, ngrok…) protected by a token or
access rule. The server calls `POST {ADVISOR_LLM_URL}/v1/chat/completions`
with a JSON schema response format, an 11 s timeout and 10 requests per
account per 10 minutes.

The rules' answer is sent to the model as a starting point. The model's
bitrate may move at most half of it either way, and reasons that are not full
sentences are replaced by the rules' reasons, so a small 3B model stays useful
(tested with `qwen2.5-vl-3b-instruct` at a 2048 context: about 6 s per answer).

Every model answer is also clamped by the same rules as the fallback (relay ≤ 15
Mb/s, HEVC only when both sides decode it, FPS ≤ both refresh rates,
resolution ≤ the viewer screen). Without a model, or when it is slow or
answers nonsense, the built-in rules answer and the app says so.
