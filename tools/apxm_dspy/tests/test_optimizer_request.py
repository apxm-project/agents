from __future__ import annotations

import json

from apxm_dspy.optimizer import _load_backend, _load_training_data
from apxm_dspy.types import BackendKey, RequestKey


def test_load_training_data_from_compiler_path(tmp_path):
    training_data = [{RequestKey.TRAINING_DATA: {"x": "y"}, "output": "z"}]
    path = tmp_path / "train.json"
    path.write_text(json.dumps(training_data))

    loaded = _load_training_data({RequestKey.TRAINING_DATA_PATH: str(path)})

    assert loaded == training_data


def test_load_backend_from_compiler_json():
    backend = {
        BackendKey.PROTOCOL: "vllm",
        BackendKey.MODEL: "registered-model",
        BackendKey.ENDPOINT: "http://localhost:8916/v1",
    }

    loaded = _load_backend({RequestKey.BACKEND_JSON: json.dumps(backend)})

    assert loaded == backend
