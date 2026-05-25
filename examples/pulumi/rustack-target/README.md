# Rustack Pulumi Target

This Pulumi program deploys a small AWS stack into a running Rustack server by
using the Pulumi AWS provider with service endpoint overrides.

```bash
make pulumi-smoke
```

The smoke target builds and starts Rustack when `http://127.0.0.1:4566` is not
already healthy, logs Pulumi into a temporary local backend, runs `pulumi up`,
prints stack outputs, and destroys the resources before exiting.

To point at an already running server:

```bash
RUSTACK_ENDPOINT=http://127.0.0.1:4566 RUSTACK_SKIP_START=1 make pulumi-smoke
```
