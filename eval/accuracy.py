from __future__ import annotations

"""Harness de acurácia do OmniCompress."""
import argparse
import json
import os
import re
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Callable

from providers import ChatProvider, OpenAICompatProvider


@dataclass
class Item:
    id: str
    context: list[dict]
    question: str
    gold: str | None = None


def load_dataset(path: str) -> list[Item]:
    """Carrega um dataset JSONL em uma lista de Item."""
    items: list[Item] = []
    with open(path, "r", encoding="utf-8") as fh:
        for raw in fh:
            line = raw.strip()
            if not line:
                continue
            obj = json.loads(line)
            items.append(
                Item(
                    id=obj.get("id", ""),
                    context=obj.get("context", []),
                    question=obj.get("question", ""),
                    gold=obj.get("gold"),
                )
            )
    return items


def _norm(s: str) -> str:
    """Normaliza para comparação: lower, strip, colapsa espaços."""
    return re.sub(r"\s+", " ", str(s).lower()).strip()


def default_tokenizer(messages: list[dict]) -> int:
    """Proxy de tokens: ~1 token a cada 4 caracteres de conteúdo."""
    return sum(len(m.get("content", "")) // 4 for m in messages)


def default_compress(messages: list[dict]) -> list[dict]:
    """Comprime mensagens via omnicompress (import lazy)."""
    try:
        import omnicompress  # type: ignore
    except ImportError as exc:
        raise RuntimeError(
            "instale o wheel omnicompress (default_compress requer o pacote)"
        ) from exc
    result = omnicompress.compress(messages)
    out = result["messages"] if isinstance(result, dict) and "messages" in result else result
    # The binding returns content-focused dicts; ensure each has a role so the
    # compressed messages remain valid for downstream chat APIs.
    return [{"role": m.get("role", "user"), "content": m.get("content", "")} for m in out]


def run(
    dataset: list[Item],
    provider: ChatProvider,
    compress_fn: Callable[[list[dict]], list[dict]] = default_compress,
    tokenizer: Callable[[list[dict]], int] = default_tokenizer,
) -> dict:
    """Executa a harness e retorna relatório estruturado."""
    items_report: list[dict] = []
    ratios: list[float] = []
    fidelity_count = 0
    correct_full_count = 0
    correct_comp_count = 0
    gold_count = 0

    for item in dataset:
        tokens_full = tokenizer(item.context)
        ans_full = provider.answer(item.context, item.question)

        comp = compress_fn(item.context)
        tokens_comp = tokenizer(comp)
        ans_comp = provider.answer(comp, item.question)

        fidelity = _norm(ans_full) == _norm(ans_comp)
        ratio = tokens_comp / max(1, tokens_full)

        if item.gold is not None:
            gold_count += 1
            correct_full = _norm(item.gold) in _norm(ans_full)
            correct_comp = _norm(item.gold) in _norm(ans_comp)
        else:
            correct_full = None
            correct_comp = None

        ratios.append(ratio)
        fidelity_count += int(fidelity)
        if correct_full is not None:
            correct_full_count += int(correct_full)
            correct_comp_count += int(correct_comp)

        items_report.append(
            {
                "id": item.id,
                "tokens_full": tokens_full,
                "tokens_comp": tokens_comp,
                "ratio": ratio,
                "fidelity": fidelity,
                "correct_full": correct_full,
                "correct_comp": correct_comp,
                "ans_full": ans_full,
                "ans_comp": ans_comp,
            }
        )

    n = len(dataset)
    mean_ratio = (sum(ratios) / n) if n else 0.0
    fidelity_rate = (fidelity_count / n) if n else 0.0
    accuracy_full = (correct_full_count / gold_count) if gold_count else None
    accuracy_comp = (correct_comp_count / gold_count) if gold_count else None
    tokens_saved_pct = 1 - mean_ratio

    summary = {
        "n": n,
        "mean_ratio": mean_ratio,
        "fidelity_rate": fidelity_rate,
        "accuracy_full": accuracy_full,
        "accuracy_comp": accuracy_comp,
        "tokens_saved_pct": tokens_saved_pct,
        "gold_count": gold_count,
    }

    return {"summary": summary, "items": items_report}


def render_markdown(report: dict) -> str:
    """Renderiza um relatório em Markdown."""
    s = report["summary"]
    saved_pct = s["tokens_saved_pct"] * 100
    fid_pct = s["fidelity_rate"] * 100
    acc_full = s["accuracy_full"]
    acc_comp = s["accuracy_comp"]
    acc_full_str = f"{acc_full * 100:.1f}%" if acc_full is not None else "—"
    acc_comp_str = f"{acc_comp * 100:.1f}%" if acc_comp is not None else "—"

    lines = [
        "# Relatório de Acurácia — OmniCompress",
        "",
        "| métrica | valor |",
        "|---|---|",
        f"| itens | {s['n']} |",
        f"| compressão média | {(1 - s['mean_ratio']) * 100:.1f}% |",
        f"| fidelidade | {fid_pct:.1f}% |",
        f"| acurácia (cheia) | {acc_full_str} |",
        f"| acurácia (comprimida) | {acc_comp_str} |",
        "",
        f"> **{saved_pct:.1f}% menos tokens, fidelidade {fid_pct:.1f}%.**",
    ]
    return "\n".join(lines)


def main(argv=None) -> None:
    """Entry point CLI para a harness."""
    parser = argparse.ArgumentParser(description="Harness de acurácia OmniCompress")
    parser.add_argument("dataset", help="caminho para dataset JSONL")
    parser.add_argument("--base-url", required=True, help="base URL do endpoint OpenAI-compat")
    parser.add_argument("--model", required=True, help="nome do modelo")
    parser.add_argument(
        "--api-key",
        default=os.environ.get("OMNICOMPRESS_EVAL_API_KEY"),
        help="chave de API (default: env OMNICOMPRESS_EVAL_API_KEY)",
    )
    parser.add_argument(
        "--report-out",
        default="eval-report.json",
        help="caminho do relatório JSON (default: eval-report.json)",
    )
    args = parser.parse_args(argv)

    dataset = load_dataset(args.dataset)
    provider = OpenAICompatProvider(
        base_url=args.base_url,
        model=args.model,
        api_key=args.api_key,
    )
    report = run(dataset, provider)

    out_path = Path(args.report_out)
    out_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, default=str),
        encoding="utf-8",
    )
    print(render_markdown(report))


if __name__ == "__main__":
    main()
