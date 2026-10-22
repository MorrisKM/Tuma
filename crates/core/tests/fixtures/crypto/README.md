Test-only RSA keys and messages for `tests/crypto.rs`, made with OpenSSL. These keys protect nothing.

- `test-private.pem` (PKCS#8), `test-private-pkcs1.pem` (the same key as PKCS#1), `test-public.pem` (SPKI)
- `hybrid-response.txt`: base64 of `RSA-OAEP(base64 AES key):RSA-OAEP(base64 IV):AES-256-CBC(JSON)`, OAEP with SHA-1
- `pin-oaep-sha1.txt`: the PIN 1234, RSA-OAEP with SHA-1 and MGF1-SHA-1, base64
