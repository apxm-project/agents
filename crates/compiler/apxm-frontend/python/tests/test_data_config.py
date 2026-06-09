import os


def test_resolve_data_layout_includes_extra_cache_and_model_roots(tmp_path, monkeypatch):
    from apxm.data_config import resolve_data_layout

    repo = tmp_path / "repo"
    config_dir = repo / ".apxm"
    config_dir.mkdir(parents=True)
    (repo / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (repo / "crates").mkdir()
    (config_dir / "config.toml").write_text(
        """
schema_version = 1

[data]
dir = "$APXM_TEST_SHARED/base"

[data.vllm]
hf_cache = "$APXM_TEST_SHARED/hf-primary"
hf_cache_roots = ["$APXM_TEST_SHARED/hf-extra", "$APXM_TEST_SHARED/hf-primary"]
model_roots = ["$APXM_TEST_SHARED/models", "$APXM_TEST_SHARED/models"]
image_store = "$APXM_TEST_SHARED/images"
""",
        encoding="utf-8",
    )

    shared = tmp_path / "shared"
    monkeypatch.setenv("APXM_TEST_SHARED", str(shared))
    layout = resolve_data_layout(repo, environ={"APXM_TEST_SHARED": str(shared)})

    assert layout.hf_cache == shared / "hf-primary"
    assert layout.hf_cache_roots == (
        shared / "hf-primary",
        shared / "hf-extra",
    )
    assert layout.model_roots == (shared / "models",)
    assert layout.image_store == shared / "images"
    assert layout.sources["hf_cache_roots"] == "config:project:data.vllm.hf_cache_roots"
    assert layout.sources["model_roots"] == "config:project:data.vllm.model_roots"


def test_resolve_data_layout_roots_can_come_from_env(tmp_path):
    from apxm.data_config import resolve_data_layout

    repo = tmp_path / "repo"
    (repo / ".apxm").mkdir(parents=True)
    (repo / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (repo / "crates").mkdir()

    first = tmp_path / "cache-a"
    second = tmp_path / "cache-b"
    model_root = tmp_path / "models"
    env = {
        "APXM_VLLM_HF_HOME": str(first),
        "APXM_VLLM_HF_CACHE_ROOTS": f"{second}{os.pathsep}{first}",
        "APXM_VLLM_MODEL_ROOTS": str(model_root),
    }

    layout = resolve_data_layout(repo, environ=env)

    assert layout.hf_cache == first
    assert layout.hf_cache_roots == (first, second)
    assert layout.model_roots == (model_root,)
    assert layout.sources["hf_cache"] == "env:APXM_VLLM_HF_HOME"
    assert layout.sources["hf_cache_roots"] == "env:APXM_VLLM_HF_CACHE_ROOTS"
    assert layout.sources["model_roots"] == "env:APXM_VLLM_MODEL_ROOTS"
