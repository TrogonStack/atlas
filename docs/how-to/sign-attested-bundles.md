# Sign attested bundles

Give the server an Ed25519 key so its attested bundles carry a signature,
and give verifiers the public key so they can check it. For what a
signature proves, see
[integrity, not authenticity](../explanation/attested-export.md#integrity-not-authenticity).

## Create the key pair

Generate the key where it will live, with any tool that writes PKCS#8 PEM.
With OpenSSL:

```sh
openssl genpkey -algorithm ed25519 -out atlas-signing.pem
chmod 600 atlas-signing.pem
openssl pkey -in atlas-signing.pem -pubout -out atlas-signing.pub.pem
```

The server never generates or stores a key. Keep `atlas-signing.pem` in
whatever holds the server's other secrets, and publish
`atlas-signing.pub.pem` through a channel verifiers already trust, such as
a release repository or an internal key directory.

The key id a bundle carries is the SHA-256 of the raw 32-byte public key.
To compute it from the public PEM:

```sh
openssl pkey -pubin -in atlas-signing.pub.pem -outform DER | tail -c 32 | shasum -a 256
```

## Point the server at the key

Name the key by file:

```sh
trogon-atlas-server --attested-signing-key-file /run/secrets/atlas-signing.pem
```

or by the environment variable that holds it, when the secret store injects
keys as variables:

```sh
export ATLAS_SIGNING_KEY="$(cat atlas-signing.pem)"
trogon-atlas-server --attested-signing-key-env ATLAS_SIGNING_KEY
```

`--attested-signing-key-env` takes the variable's name, never the key. The
matching settings `TROGON_ATLAS_ATTESTED_SIGNING_KEY_FILE` and
`TROGON_ATLAS_ATTESTED_SIGNING_KEY_ENV` work the same way. At startup the
server logs `attested bundles will be signed` with the key id.

## Check that exports are signed

```sh
trogon-atlas export-attested shop/checkout -o checkout.bundle.json
trogon-atlas verify checkout.bundle.json --trusted-key atlas-signing.pub.pem
```

`verify` prints `signed by trusted key` and the key id. If the export is
refused with `SIGNING_KEY_UNAVAILABLE` (exit 4), the server could not read
or parse its key; its log names the file or variable that failed. The
server refuses rather than return an unsigned bundle.

## Rotate the key

The server reads the key on every export, so replacing the file or the
variable's value takes effect on the next export without a restart. A
variable's value only changes when the process restarts with the new one.

1. Publish the new public key next to the old one.
2. Have verifiers pass both, with `--trusted-key` once per key.
3. Replace the server's key.
4. Once no bundle signed by the old key still needs checking, retire its
   public key.

Bundles signed by the old key keep verifying for any verifier still passing
the old public key.
