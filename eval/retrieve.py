import json
import os
import argparse
import re
import urllib.request
import urllib.error
from typing import Callable

from accuracy import Item, load_dataset, _norm, default_tokenizer
from providers import OpenAICompatProvider


class ToolChatProvider:
    """OpenAI-compatible chat provider with a retrieve tool-call loop."""

    def __init__(
        self,
        base_url: str,
        model: str,
        retrieve_fn: Callable[[str], str],
        api_key: str | None = None,
        timeout: float = 90.0,
        max_rounds: int = 4,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.retrieve_fn = retrieve_fn
        self.api_key = api_key
        self.timeout = timeout
        self.max_rounds = max_rounds

    @staticmethod
    def _sanitize(messages: list[dict]) -> list[dict]:
        out = []
        for m in messages:
            role = m.get("role", "user")
            if role not in {"system", "user", "assistant"}:
                role = "user"
            out.append({"role": role, "content": m.get("content", "")})
        return out

    def answer(self, messages: list[dict], question: str) -> tuple[str, int]:
        convo = self._sanitize(messages) + [{"role": "user", "content": question}]
        tool = {
            "type": "function",
            "function": {
                "name": "retrieve",
                "description": (
                    "Expand an omnicompress-compressed block to its full original "
                    "content, given its CCR hash (shown in the "
                    "[omnicompress: ... hash=...] marker)."
                ),
                "parameters": {
                    "type": "object",
                    "properties": {"hash": {"type": "string"}},
                    "required": ["hash"],
                },
            },
        }
        n_retrieves = 0
        last_content = ""
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"

        for _ in range(self.max_rounds):
            payload = {
                "model": self.model,
                "messages": convo,
                "tools": [tool],
                "temperature": 0,
            }
            data = json.dumps(payload).encode("utf-8")
            req = urllib.request.Request(
                f"{self.base_url}/chat/completions",
                data=data,
                headers=headers,
                method="POST",
            )
            try:
                with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                    body = json.loads(resp.read().decode("utf-8"))
            except (urllib.error.URLError, OSError, json.JSONDecodeError) as exc:
                raise RuntimeError(f"retrieve chat request failed: {exc}") from exc

            msg = body["choices"][0]["message"]
            last_content = str(msg.get("content", "") or "")
            tool_calls = msg.get("tool_calls")
            if tool_calls:
                convo.append(msg)
                for tc in tool_calls:
                    fn = tc.get("function", {})
                    if fn.get("name") != "retrieve":
                        continue
                    try:
                        args = json.loads(fn.get("arguments", "{}"))
                    except json.JSONDecodeError:
                        args = {}
                    h = args.get("hash", "")
                    original = self.retrieve_fn(h)
                    convo.append(
                        {
                            "role": "tool",
                            "tool_call_id": tc.get("id", ""),
                            "content": original,
                        }
                    )
                    n_retrieves += 1
                continue
            return (last_content.strip(), n_retrieves)

        return (last_content.strip(), n_retrieves)


class MockToolProvider:
    """In-memory tool-call provider driven by a script (no network)."""

    _HASH_RE = re.compile(r"hash=([0-9a-f]+)")

    def __init__(self, script: list[dict]) -> None:
        self.script = script
        self.retrieve_fn: Callable[[str], str] | None = None

    def _concat(self, messages: list[dict]) -> str:
        return "\n".join(m.get("content", "") for m in messages)

    def answer(self, messages: list[dict], question: str) -> tuple[str, int]:
        n_retrieves = 0
        convo = list(messages) + [{"role": "user", "content": question}]
        text = self._concat(convo)
        hashes = self._HASH_RE.findall(text)
        for action in self.script:
            if "retrieve_hash" in action:
                substr = action["retrieve_hash"]
                full = next((h for h in hashes if substr in h), None)
                if full is None:
                    continue
                if self.retrieve_fn is None:
                    raise RuntimeError("MockToolProvider.retrieve_fn not set")
                original = self.retrieve_fn(full)
                convo.append({"role": "tool", "content": original})
                n_retrieves += 1
            elif "answer" in action:
                return (str(action["answer"]), n_retrieves)
        return ("", n_retrieves)


def retrieve_aware_run(
    dataset,
    base_url: str,
    model: str,
    api_key: str | None = None,
    tokenizer=default_tokenizer,
) -> dict:
    """Run baseline vs compressed-with-retrieve evaluation."""
    import omnicompress

    items: list[Item] = []
    provider_full = OpenAICompatProvider(base_url, model, api_key)

    per_item = []
    sum_ratio = 0.0
    fidelity_hits = 0
    acc_full_hits = 0
    acc_comp_hits = 0
    retrieve_items = 0
    sum_tokens_full = 0
    sum_tokens_comp = 0

    for item in dataset:
        items.append(item)
        sess = omnicompress.OmniCompressSession()
        comp = sess.compress(item.context)
        comp_msgs = [
            {"role": m.get("role", "user"), "content": m.get("content", "")}
            for m in comp["messages"]
        ]
        tokens_full = tokenizer(item.context)
        tokens_comp = tokenizer(comp_msgs)

        ans_full = provider_full.answer(item.context, item.question)
        prov = ToolChatProvider(
            base_url, model, retrieve_fn=sess.retrieve, api_key=api_key
        )
        ans_comp, n_ret = prov.answer(comp_msgs, item.question)

        ratio = (tokens_comp / tokens_full) if tokens_full else 0.0
        sum_ratio += ratio
        sum_tokens_full += tokens_full
        sum_tokens_comp += tokens_comp

        n_full = _norm(ans_full)
        n_comp = _norm(ans_comp)
        n_gold = _norm(item.gold) if item.gold else ""
        fid = n_full == n_comp
        if fid:
            fidelity_hits += 1
        correct_full = bool(n_gold) and n_gold in n_full
        correct_comp = bool(n_gold) and n_gold in n_comp
        if correct_full:
            acc_full_hits += 1
        if correct_comp:
            acc_comp_hits += 1
        if n_ret > 0:
            retrieve_items += 1

        per_item.append(
            {
                "id": item.id,
                "tokens_full": tokens_full,
                "tokens_comp": tokens_comp,
                "ratio": ratio,
                "fidelity": fid,
                "correct_full": correct_full,
                "correct_comp": correct_comp,
                "n_retrieves": n_ret,
                "ans_full": ans_full,
                "ans_comp": ans_comp,
            }
        )

    n = len(items)
    mean_ratio = (sum_ratio / n) if n else 0.0
    fidelity_rate = (fidelity_hits / n) if n else 0.0
    accuracy_full = (acc_full_hits / n) if n else 0.0
    accuracy_comp = (acc_comp_hits / n) if n else 0.0
    retrieve_rate = (retrieve_items / n) if n else 0.0
    tokens_saved_pct = (
        (1.0 - (sum_tokens_comp / sum_tokens_full)) * 100.0 if sum_tokens_full else 0.0
    )

    return {
        "summary": {
            "n": n,
            "mean_ratio": mean_ratio,
            "fidelity_rate": fidelity_rate,
            "accuracy_full": accuracy_full,
            "accuracy_comp": accuracy_comp,
            "retrieve_rate": retrieve_rate,
            "tokens_saved_pct": tokens_saved_pct,
        },
        "items": per_item,
    }


def render_markdown(report: dict) -> str:
    s = report["summary"]
    lines = [
        "# Eval: retrieve-aware",
        "",
        "| itens | compressão % | retrieve_rate % | acurácia cheia | acurácia comprimida-com-retrieve |",
        "|---|---|---|---|---|",
        f"| {s['n']} | {s['tokens_saved_pct']:.1f} | {s['retrieve_rate']*100:.1f} | "
        f"{s['accuracy_full']*100:.1f} | {s['accuracy_comp']*100:.1f} |",
        "",
        f"compressão {s['tokens_saved_pct']:.1f}% · acurácia recuperada "
        f"{s['accuracy_comp']*100:.1f}% com retrieve em "
        f"{s['retrieve_rate']*100:.1f}% dos casos",
    ]
    return "\n".join(lines)


def main(argv=None) -> None:
    parser = argparse.ArgumentParser(description="Retrieve-aware accuracy harness")
    parser.add_argument("dataset", help="Path to dataset JSONL")
    parser.add_argument("--base-url", required=True, help="OpenAI-compatible base URL")
    parser.add_argument("--model", required=True, help="Model name")
    parser.add_argument(
        "--api-key",
        default=os.environ.get("OMNICOMPRESS_EVAL_API_KEY"),
        help="API key (default: env OMNICOMPRESS_EVAL_API_KEY)",
    )
    parser.add_argument(
        "--report-out",
        default="eval-retrieve-report.json",
        help="Path to write JSON report",
    )
    args = parser.parse_args(argv)

    dataset = load_dataset(args.dataset)
    report = retrieve_aware_run(dataset, args.base_url, args.model, api_key=args.api_key)
    with open(args.report_out, "w", encoding="utf-8") as fh:
        json.dump(report, fh, ensure_ascii=False, indent=2)
    print(render_markdown(report))


if __name__ == "__main__":
    main()
