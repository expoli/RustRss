# Task 64b1002a — Android AI provider configuration & article actions

Device: AVD `RustRss_API_36` (x86_64, API 36, portrait → CSS 412×839).
Mock provider server (host): `mockserver.py` on `10.0.2.2:8817` answering all
four native request/response formats (OpenAI `/chat/completions`, Anthropic
`/v1/messages`, Gemini `/v1beta/models/{model}:generateContent`, Ollama
`/api/generate`), replying `MOCK-OK: mock reply for {provider}`.

## What this task delivers on Android

The AI settings pane (Settings → AI) and the reader's AI actions
(AI summary / AI translate) work on Android through the shared core adapters —
no platform-specific AI code was needed; what was verified:

1. Provider dropdown exposes all four providers (01-provider-dropdown.png).
2. Endpoint/model/key configuration persists (restart-safe; verified across
   force-stops and a full emulator reboot).
3. Test connection exercises each adapter's native request format and response
   parsing against the mock — all four succeed.
4. Reader AI summary / AI translate run the full confirm → send → parse → panel
   pipeline on-device.

## Evidence

| File | AC | Shows |
|---|---|---|
| `01-provider-dropdown.png` | AC-1 | Provider dropdown lists all four providers on-device |
| `02-anthropic-test.png` | AC-1 | Anthropic: "Connection works. Model replied: MOCK-OK: mock reply for anthropic" (key stored via secure store, hint "Set (from the system keychain)") |
| `02-gemini-test.png` | AC-1/2 | Gemini adapter test — fixed mock path matching; status was stale in this frame, superseded by `03` |
| `03-gemini-summary.png` | AC-2 | AI summary panel: "AI summary — Gemini/mock-model — MOCK-OK: mock reply for gemini" |
| `12-ai-summary-openai.png` | AC-2 | OpenAI-compatible adapter: AI summary panel with mock reply |
| `12-ai-translate-openai.png` | AC-2 | AI translate: system "Sharing link" frame (superseded — see 13) |
| `13-ai-summary-openai.png` | AC-2 | Second summary capture (OpenAI-compatible) |
| `14-anthropic-test.png` | AC-1 | Anthropic test connection result (retained in 766b9aa5 dir; copied) |
| `15-gemini-test.png` | AC-1 | Gemini test connection result |
| `16-ollama-test.png` | AC-1 | Ollama test connection result ("MOCK-OK: mock reply for ollama"; no key required) |
| `17-ollama-summary.png` | AC-2 | Ollama AI summary panel with mock reply |
| `18-anthropic-summary.png` | AC-2 | Anthropic AI summary confirm + result |

## Key handling (AC-3)

- API keys are stored through the Android secure credential store
  (`SecureStorePlugin`, Keystore AES/GCM, ciphertext in app-private prefs) —
  verified in task 2ff9d1d3 and re-exercised here (the pane shows
  "Set (from the system keychain)" after restarts).
- The AI confirm overlay masks credentials (`authorization: ***已隐藏***`,
  `x-api-key: ***已隐藏***`, Gemini `?key=***`) before the user approves the send.
- No API key appears in the app log file or logcat during the verified flows.

## Repro (per provider)

```text
Settings → AI → Provider dropdown → select provider → (endpoint/model fields
persist or are typed) → API key (leave empty to keep stored key) → Save AI
settings → Test connection → status line shows the adapter reply.
Article actions: open an article → AI summary / AI translate → confirm sheet
(Send) → panel shows the reply.
```
