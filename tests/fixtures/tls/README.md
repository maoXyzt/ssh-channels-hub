These self-signed localhost certificate and PKCS#8 key fixtures are for tests only.
The certificate has DNS SAN localhost and is valid for 100 years from November 2026.
The integration test trusts only this certificate, retaining rustls certificate
and hostname verification. The private key is public test data, never a service key.
