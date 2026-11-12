import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
CHART = REPO / "devops/helm/charts/trogon-atlas-server"
PREFIX = "TROGON_ATLAS_ARTIFACT_"


class ArtifactValuesTest(unittest.TestCase):
    def render(self, artifacts, max_message_bytes=37748736):
        values = {
            "auth": {"insecureAllowAnonymous": True},
            "config": {"maxMessageBytes": max_message_bytes},
            "artifacts": artifacts,
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "values.json"
            path.write_text(json.dumps(values))
            return subprocess.run(
                [
                    "helm",
                    "template",
                    "artifact-values",
                    str(CHART),
                    "--show-only",
                    "templates/deployment.yaml",
                    "--values",
                    str(path),
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=30,
            )

    def environment(self, artifacts, prefix=PREFIX):
        rendered = self.render(artifacts)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        decoded = subprocess.run(
            [
                "yq",
                "-o=json",
                '.spec.template.spec.containers[] | select(.name == "server") | .env',
            ],
            input=rendered.stdout,
            capture_output=True,
            text=True,
            check=True,
            timeout=10,
        )
        return {
            item["name"].removeprefix(prefix): item["value"]
            for item in json.loads(decoded.stdout)
            if item["name"].startswith(prefix)
        }

    @staticmethod
    def enabled(kind, spec):
        return {
            "backend": {"kind": kind, "spec": spec},
            "catalog": {"natsKvBucket": "fixture_artifact_catalog"},
            "maxBytes": 1048576,
            "uploadTtlSecs": 300,
        }

    def assert_invalid(self, artifacts):
        rendered = self.render(artifacts)
        self.assertNotEqual(rendered.returncode, 0)
        self.assertIn("schema", rendered.stderr)
        self.assertIn("artifacts", rendered.stderr)

    def test_disabled_backend_has_no_storage_configuration(self):
        self.assertEqual(self.environment({}), {"BACKEND": "disabled"})

    def test_nats_selects_only_object_store_and_shared_catalog(self):
        self.assertEqual(
            self.environment(
                self.enabled("nats", {"bucket": "fixture_artifact_bytes"})
            ),
            {
                "BACKEND": "nats",
                "NATS_BUCKET": "fixture_artifact_bytes",
                "CATALOG_BUCKET": "fixture_artifact_catalog",
                "MAX_BYTES": "1048576",
                "UPLOAD_TTL_SECS": "300",
            },
        )

    def test_s3_selects_only_s3_and_shared_nats_catalog(self):
        self.assertEqual(
            self.environment(
                self.enabled(
                    "s3",
                    {
                        "bucket": "fixture-artifact-bytes",
                        "region": "eu-west-1",
                        "endpoint": "https://objects.example.com",
                        "forcePathStyle": True,
                        "allowHttp": True,
                    },
                )
            ),
            {
                "BACKEND": "s3",
                "CATALOG_BUCKET": "fixture_artifact_catalog",
                "MAX_BYTES": "1048576",
                "UPLOAD_TTL_SECS": "300",
                "S3_BUCKET": "fixture-artifact-bytes",
                "S3_REGION": "eu-west-1",
                "S3_ENDPOINT": "https://objects.example.com",
                "S3_FORCE_PATH_STYLE": "true",
                "S3_ALLOW_HTTP": "true",
            },
        )

    def test_s3_defaults_region_and_transport_options(self):
        env = self.environment(self.enabled("s3", {"bucket": "fixture-artifact-bytes"}))
        self.assertEqual(env["S3_REGION"], "us-east-1")
        self.assertEqual(env["S3_FORCE_PATH_STYLE"], "false")
        self.assertEqual(env["S3_ALLOW_HTTP"], "false")
        self.assertEqual(env["CATALOG_BUCKET"], "fixture_artifact_catalog")
        self.assertNotIn("S3_ENDPOINT", env)
        self.assertNotIn("NATS_BUCKET", env)

    def test_public_url_configuration_is_rejected_for_every_backend(self):
        for kind, spec in [
            ("disabled", {}),
            ("nats", {"bucket": "fixture"}),
            ("s3", {"bucket": "fixture"}),
        ]:
            for public_url in ["", "https://atlas.example.com", 42, None]:
                artifacts = self.enabled(kind, spec)
                artifacts["publicUrl"] = public_url
                with self.subTest(kind=kind, public_url=public_url):
                    self.assert_invalid(artifacts)

    def test_no_backend_emits_a_public_url(self):
        for artifacts in [
            {},
            self.enabled("nats", {"bucket": "fixture"}),
            self.enabled("s3", {"bucket": "fixture"}),
        ]:
            with self.subTest(artifacts=artifacts):
                self.assertNotIn("PUBLIC_URL", self.environment(artifacts))

    def test_transfer_limits_remain_decimal_in_rendered_environment(self):
        env = self.environment(self.enabled("nats", {"bucket": "fixture"}), prefix="")
        self.assertEqual(env["TROGON_ATLAS_MAX_MESSAGE_BYTES"], "37748736")
        self.assertEqual(env["TROGON_ATLAS_ARTIFACT_MAX_BYTES"], "1048576")
        self.assertEqual(env["TROGON_ATLAS_ARTIFACT_UPLOAD_TTL_SECS"], "300")

    def test_enabled_backends_enforce_message_size_boundary(self):
        for kind in ["nats", "s3"]:
            artifacts = self.enabled(kind, {"bucket": "fixture"})
            boundary = artifacts["maxBytes"] + 65536
            with self.subTest(kind=kind):
                accepted = self.render(artifacts, max_message_bytes=boundary)
                self.assertEqual(accepted.returncode, 0, accepted.stderr)
                rejected = self.render(artifacts, max_message_bytes=boundary - 1)
                self.assertNotEqual(rejected.returncode, 0)
                self.assertIn("maxMessageBytes", rejected.stderr)
                self.assertIn("artifacts.maxBytes", rejected.stderr)

    def test_unknown_backend_kind_is_rejected(self):
        self.assert_invalid(self.enabled("unknown", {}))

    def test_active_backends_require_nonempty_bucket(self):
        for kind in ["nats", "s3"]:
            for spec in [{}, {"bucket": ""}]:
                with self.subTest(kind=kind, spec=spec):
                    self.assert_invalid(self.enabled(kind, spec))

    def test_provider_fields_cannot_cross_backend_boundary(self):
        for kind, spec in [
            ("nats", {"bucket": "fixture", "region": "eu-west-1"}),
            ("nats", {"bucket": "fixture", "endpoint": "https://objects.example.com"}),
            ("s3", {"bucket": "fixture", "natsBucket": "another"}),
        ]:
            with self.subTest(kind=kind, spec=spec):
                self.assert_invalid(self.enabled(kind, spec))

    def test_disabled_backend_rejects_provider_spec(self):
        self.assert_invalid(
            {"backend": {"kind": "disabled", "spec": {"bucket": "fixture"}}}
        )

    def test_legacy_backend_value_wrapper_is_rejected(self):
        for kind, config in [
            ("disabled", {}),
            ("nats", {"bucket": "fixture"}),
            ("s3", {"bucket": "fixture"}),
        ]:
            for backend in [
                {"kind": kind, "value": config},
                {"kind": kind, "spec": config, "value": config},
            ]:
                artifacts = self.enabled(kind, config)
                artifacts["backend"] = backend
                with self.subTest(backend=backend):
                    self.assert_invalid(artifacts)

    def test_legacy_and_unknown_configuration_is_rejected(self):
        for artifacts in [
            {"backend": "nats", "natsBucket": "fixture"},
            {"natsBucket": "fixture"},
            {"catalogBucket": "fixture"},
            {"s3": {"bucket": "fixture"}},
            {
                "backend": {
                    "kind": "nats",
                    "spec": {"bucket": "fixture"},
                    "extra": True,
                }
            },
            {"catalog": {"bucket": "fixture"}},
        ]:
            with self.subTest(artifacts=artifacts):
                self.assert_invalid(artifacts)

    def test_shared_catalog_is_required_and_typed(self):
        for catalog in [{"natsKvBucket": ""}, {"natsKvBucket": 42}]:
            artifacts = self.enabled("s3", {"bucket": "fixture"})
            artifacts["catalog"] = catalog
            with self.subTest(catalog=catalog):
                self.assert_invalid(artifacts)

    def test_shared_limits_remain_bounded_integers(self):
        for field, invalid_values in [
            ("maxBytes", [0, 33554433, 1.5, True, "1048576"]),
            ("uploadTtlSecs", [0, 3601, 2.5, False, "300"]),
        ]:
            for value in invalid_values:
                artifacts = self.enabled("nats", {"bucket": "fixture"})
                artifacts[field] = value
                with self.subTest(field=field, value=value):
                    self.assert_invalid(artifacts)


if __name__ == "__main__":
    unittest.main()
