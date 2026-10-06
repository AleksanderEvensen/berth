These are public, disposable credentials for the in-process TLS test server.
They must never be used for a real engine or other service. No production secret
is present here. The test CA private key was discarded after signing these
fixtures. Certificates expire in 2126 so tests do not depend on short lifetimes.

The server certificate covers only `localhost`, deliberately excluding the
loopback IP address so the test can exercise `TlsConfig.verify = false`.
The client certificate has the clientAuth usage; the test server requires it.
