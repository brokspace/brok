# Native clients for non OpenAI-compatible LLM APIs

The core (`brok` and `brok-agent`) talks to LLMs through one interface: the OpenAI-compatible chat completions API. Requests to add native clients for provider-specific APIs are declined.

## Why this is out of scope

One well-supported interface covers the providers Brokspace users actually reach for: Ollama and LM Studio locally, and hosted providers such as Anthropic, DeepSeek, OpenAI, Groq, OpenRouter and many more, which all expose OpenAI-compatible endpoints. Each additional native client is a second code path for streaming, errors, auth and retries that every change to the agent must keep working.

Providers without an OpenAI-compatible endpoint can still be used:

- through a local proxy that speaks the OpenAI API (LiteLLM and similar), configured as a normal provider in `brok.yml`;
- through a shard that subscribes to `agent:ask` itself and talks to whatever API it likes.

## Prior requests

None yet.
