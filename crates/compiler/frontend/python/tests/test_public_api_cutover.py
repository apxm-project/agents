"""Public API hard-cutover checks for generic frontend primitives."""

import apxm


def test_core_package_does_not_export_conversational_abstractions():
    for name in ("ConversationalAgent", "CompactionPolicy", "MultiFlowArtifact"):
        assert not hasattr(apxm, name), f"apxm.{name} must not be a core API export"
        assert name not in apxm.__all__


def test_graph_recorder_has_no_call_skill_builder():
    assert not hasattr(apxm.GraphRecorder, "call_skill")
