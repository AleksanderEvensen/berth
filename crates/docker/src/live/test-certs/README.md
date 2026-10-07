These are public, disposable credentials from a throwaway test CA. They must
never be used for a real engine or other service. No production secret is
present here. The test CA private key was discarded after signing these
fixtures. Certificates expire in 2126 so tests do not depend on short lifetimes.

Tests use them as *foreign* credentials: a CA the daemon's certificate isn't
signed by, and a client certificate (clientAuth usage) the daemon must reject.
See `tests/live/tls.rs`, which runs against a real TLS daemon (`docker:dind`).
