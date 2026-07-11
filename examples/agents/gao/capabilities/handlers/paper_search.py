from __future__ import annotations

import json
from pathlib import Path

from apxm import tool


CORPUS_PATH = Path(__file__).resolve().parents[2] / "shared" / "papers.json"
STOPWORDS = frozenset({"about", "and", "are", "does", "for", "how", "the", "what", "with"})


@tool(name="search_papers")
def search_papers(query: str) -> str:
    """Search the local APXM paper corpus and return cited matches."""

    terms = {
        token.strip(".,:;!?()[]{}").lower()
        for token in query.split()
        if len(token.strip(".,:;!?()[]{}")) > 2
        and token.strip(".,:;!?()[]{}").lower() not in STOPWORDS
    }
    papers = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    matches = []
    for paper in papers:
        text = " ".join(
            [
                str(paper["title"]),
                str(paper["abstract"]),
                " ".join(str(keyword) for keyword in paper["keywords"]),
            ]
        ).lower()
        score = sum(term in text for term in terms)
        if score:
            matches.append(
                {
                    "id": paper["id"],
                    "citation": paper["citation"],
                    "title": paper["title"],
                    "abstract": paper["abstract"],
                    "score": score,
                }
            )
    matches.sort(key=lambda item: (-item["score"], item["id"]))
    return json.dumps(
        {
            "query": query,
            "matches": matches,
            "message": "" if matches else "No source found in the local paper corpus.",
        },
        sort_keys=True,
    )
