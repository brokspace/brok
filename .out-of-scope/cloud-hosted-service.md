# A hosted Brokspace service in the core

Brokspace does not include a hosted SaaS component: no Brokspace cloud account, hosted index, hosted bus or telemetry backend in the core repositories.

## Why this is out of scope

Brokspace is local-first by design. The index, the embedding model and the bus run on the developer's machine, which is what makes it usable in corporate environments with strict data policies and on laptops without a network. A hosted component would add accounts, credentials, availability and data-handling obligations to a tool whose promise is "install, init, done".

Sharing what the index knows is already covered without a service: `brok setup sync` mirrors the index to storage the user controls (S3, Cloudflare R2, MinIO, Git LFS), and LLM providers are whatever endpoint the user configures. Anyone who wants a hosted offering can build it on top of the open protocol, outside the core.

## Prior requests

None yet.
