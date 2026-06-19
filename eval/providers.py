from __future__ import annotations

"""Providers de chat para a harness de acurácia do OmniCompress."""
import json
from dataclasses import dataclass
from urllib import error, request


@dataclass
class ChatProvider:
    """Interface base para um provedor de chat."""

    def answer(self, messages: list[dict], question: str) -> str:
        raise NotImplementedError("subclasses devem implementar answer()")


class OpenAICompatProvider(ChatProvider):
    """Provedor compatível com a API OpenAI /chat/completions."""

    def __init__(
        self,
        base_url: str,
        model: str,
        api_key: str | None = None,
        timeout: float = 60.0,
    ) -> None:
        self.base_url = base_url
        self.model = model
        self.api_key = api_key
        self.timeout = timeout

    @staticmethod
    def _sanitize(messages: list[dict]) -> list[dict]:
        """Coerce roles to the chat-API trio; bare `tool` roles cause 400s."""
        ok = {"system", "user", "assistant"}
        return [
            {"role": m.get("role") if m.get("role") in ok else "user",
             "content": m.get("content", "")}
            for m in messages
        ]

    def answer(self, messages: list[dict], question: str) -> str:
        payload = {
            "model": self.model,
            "messages": self._sanitize(messages) + [{"role": "user", "content": question}],
            "temperature": 0,
        }
        data = json.dumps(payload).encode("utf-8")
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        url = f"{self.base_url.rstrip('/')}/chat/completions"
        req = request.Request(url, data=data, headers=headers, method="POST")
        try:
            with request.urlopen(req, timeout=self.timeout) as resp:
                body = resp.read().decode("utf-8")
        except (error.URLError, error.HTTPError, TimeoutError) as exc:
            raise RuntimeError(f"erro de rede ao chamar {url}: {exc}") from exc
        try:
            parsed = json.loads(body)
            content = parsed["choices"][0]["message"]["content"]
        except (KeyError, IndexError, json.JSONDecodeError) as exc:
            raise RuntimeError(f"resposta inesperada de {url}: {body!r}") from exc
        return str(content).strip()


class MockProvider(ChatProvider):
    """Provedor determinístico para testes."""

    def __init__(self, fn) -> None:
        self.fn = fn

    def answer(self, messages: list[dict], question: str) -> str:
        return str(self.fn(messages, question)).strip()
