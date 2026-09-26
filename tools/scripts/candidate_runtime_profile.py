"""Calculate an offline Runtime profile reference from verified candidate bytes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import service_images


def main() -> int:
    receipt_value = os.environ.get("APXM_CANDIDATE_RECEIPT", "")
    if not receipt_value:
        raise service_images.ImageError("APXM_CANDIDATE_RECEIPT is required")
    root = Path(__file__).resolve().parents[2]
    receipt = Path(receipt_value).resolve()
    verification = service_images.verify_candidate_images(root, receipt)
    if not verification["qualified"]:
        raise service_images.ImageError("candidate image verification failed")
    candidate = json.loads(receipt.read_text())
    runtime = next(image for image in candidate["images"] if image["service"] == "runtime-service")
    verified_runtime = next(image for image in verification["images"] if image["service"] == "runtime-service")
    image_id = verified_runtime["image_id"]
    with tempfile.TemporaryDirectory(prefix="apxm-runtime-profile-") as temporary:
        staging = Path(temporary)
        release = staging / "release.json"
        provenance = staging / "provenance.json"
        service_images._extract(
            image_id,
            {
                "/workspace/contracts/services/manifests/apxm.agents-service-release-manifest.v1.json": release,
                "/workspace/deploy/services/source-revision.v1.json": provenance,
            },
        )
        environment = os.environ.copy()
        environment.update(
            APXM_RUNTIME_CAPABILITY_PROFILE="host_only",
            APXM_RUNTIME_RELEASE_MANIFEST_PATH=str(release),
            APXM_RUNTIME_PROVENANCE_PATH=str(provenance),
        )
        result = subprocess.run(
            ["dekk", "agents", "runtime-profile-ref"],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        profile = json.loads(result.stdout)
        print(json.dumps({
            "candidate_receipt": str(receipt),
            "source_tree_digest": candidate["source"]["source_tree_digest"],
            "runtime_tag": runtime["tag"],
            "runtime_image_id": image_id,
            "runtime_service_digest": runtime["service_digest"],
            "release_carrier_digest": "sha256:" + hashlib.sha256(release.read_bytes()).hexdigest(),
            "provenance_carrier_digest": "sha256:" + hashlib.sha256(provenance.read_bytes()).hexdigest(),
            **profile,
        }, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, ValueError, subprocess.CalledProcessError, service_images.ImageError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1) from error
