# Try SAT with Pi

Pi needs a language-model provider, and SAT provides tools Pi can call. They
are separate services: the model provider charges for the conversation and
tool decisions; Brave charges for web-search usage under its own API account.

This repository already includes a Pi extension that exposes a narrow
`web_search(query)` tool and invokes SAT's registered Brave component. You do
not need to build a separate web-search harness. See
[`pi-extension/README.md`](pi-extension/README.md) for its setup and offline
checks.

## Model provider recommendation

For an inexpensive first Pi trial with open-weight models served in Europe,
**Scaleway Generative APIs Serverless** is a reasonable starting point. Its
catalog lists current open-weight chat and coding models, offers an
OpenAI-compatible API, and currently includes a free first million tokens.
For example, the published price for `qwen3.5-397b-a17b` is €0.60 per million
input tokens and €3.60 per million output tokens; `deepseek-v4-flash-0731` is
listed at €0.40 / €0.80. Prices are before tax and can change. The service says
its models are hosted in Paris, France. Check the live catalog and model page
before choosing a model.

**Payment caveat:** Scaleway's documented methods are bank cards and SEPA, not
PayPal. I could not verify a provider that is both clearly EU-based/EU-hosted,
offers low-cost current open-weight inference, and explicitly accepts PayPal
wallet payments. If PayPal is mandatory, treat that as a separate provider
search and verify the operator, model routing/data location, and whether
checkout accepts a PayPal account rather than merely processing cards through
PayPal. Some gateways advertise PayPal but are not European inference
providers.

Scaleway references:

- [Generative API pricing and model catalog](https://www.scaleway.com/en/pricing/model-as-a-service/)
- [Serverless API overview and Paris hosting](https://www.scaleway.com/en/generative-apis/)
- [Accepted payment methods](https://www.scaleway.com/en/docs/billing/faq/)

The service's free tier and payment requirements are subject to change. The
current billing FAQ says to add a valid card before ordering resources; the
Generative API pricing page describes one million free tokens. Confirm current
account requirements in the console before relying on the free tier.

## Configure Pi for Scaleway

Create a Scaleway project and API key in the Scaleway console. Keep the key in
your environment or Pi's credential store; do not commit it. The Scaleway API
uses a project-specific endpoint of the form
`https://api.scaleway.ai/<PROJECT_ID>/v1`.

Add a custom OpenAI-compatible provider to `~/.pi/agent/models.json` (merge this
with any existing configuration). Pi's `models-store.json` is a managed cache
for provider model catalogs; don't create your custom provider there. The
current Pi docs still direct custom endpoint configuration to `models.json`.
Pi 1.x may manage the catalog cache as part of its updated model runtime, but
the user configuration remains `models.json`.

```json
{
  "providers": {
    "scaleway": {
      "baseUrl": "https://api.scaleway.ai/YOUR_PROJECT_ID/v1",
      "api": "openai-completions",
      "apiKey": "$SCW_SECRET_KEY",
      "models": [
        {
          "id": "qwen3.5-397b-a17b",
          "name": "Qwen 3.5 397B (Scaleway)",
          "reasoning": true,
          "input": ["text", "image"],
          "contextWindow": 128000,
          "maxTokens": 8192,
          "cost": {
            "input": 0.6,
            "output": 3.6,
            "cacheRead": 0,
            "cacheWrite": 0
          }
        }
      ]
    }
  }
}
```

Set the key in the shell before launching Pi, then choose the model with `/model`:

```sh
export SCW_SECRET_KEY='YOUR_SCALEWAY_API_KEY'
pi --extension ./pi-extension/sat.ts
```

Pi supports custom providers through `~/.pi/agent/models.json`; the `apiKey`
field can reference an environment variable. Model IDs and supported API
features should be confirmed against Scaleway's current catalog. If this model
does not work with Pi's tool calling, try another listed chat model or check
Scaleway's compatibility notes.

Pi references:

- [Pi custom model configuration](https://pi.dev/docs/latest/models)
- [Pi custom providers](https://pi.dev/docs/latest/custom-provider)
- [Scaleway Generative API quickstart](https://www.scaleway.com/en/docs/generative-apis/quickstart/)

## Set up SAT's Brave Search tool

From the repository root:

```sh
python3 pi-extension/setup.py
npm ci --ignore-scripts --prefix pi-extension
export SAT_BIN="$PWD/sat/target/debug/sat"
export SAT_HOME="$PWD/sat/target/demo/registry"
```

For live search, set `BRAVE_API_KEY` in the host environment using a secret
manager or a hidden prompt. SAT reads it in the trusted host and inserts the
Brave credential only after enforcing the package's route policy:

```sh
read -rsp 'Brave API key: ' BRAVE_API_KEY
export BRAVE_API_KEY
pi --extension ./pi-extension/sat.ts
```

Pi can then call `web_search` when appropriate. The model only supplies a
query. The extension does not let the model pass filesystem grants or arbitrary
SAT arguments. SAT limits Brave requests to the configured HTTPS search route.
Live searches consume Brave API quota.

To run Pi without a live Brave key, use the documented offline SAT component
test first. That verifies the actual component and host policy with a synthetic
response; it does not require a model-provider key. A real Pi conversation
still needs a model endpoint and credential.

## Suggested first trial

1. Configure Pi's model provider and verify a normal prompt works.
2. Run the Pi extension's type check and tests:

   ```sh
   npm run check --prefix pi-extension
   npm test --prefix pi-extension
   ```

3. Run SAT's offline Brave component test as described in
   [`pi-extension/README.md`](pi-extension/README.md).
4. For a live end-to-end trial, set both provider keys and ask Pi to search for
   a topic. Confirm the model calls `web_search` and summarizes the returned
   titles, links, and snippets.

This first integration demonstrates Pi calling SAT through a local CLI
extension. It does not yet establish a generic SAT tool-discovery protocol or
cover the full range of filesystem grants and pipelines.
