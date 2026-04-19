"""Mock vLLM server — serves OpenAI-compatible /v1/models, /v1/chat/completions.

Used by Gate 0 to validate orchestration plumbing without real GPU inference.
"""

from __future__ import annotations

import asyncio
import random
import time
import uuid
from typing import Any

from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI(title="pantheon-mock-vllm")


class ChatMessage(BaseModel):
    role: str
    content: str


class ChatCompletionRequest(BaseModel):
    model: str
    messages: list[ChatMessage]
    temperature: float = 0.7
    max_tokens: int | None = None
    stream: bool = False


class ChatCompletionChoice(BaseModel):
    index: int
    message: ChatMessage
    finish_reason: str = "stop"


class ChatCompletionUsage(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ChatCompletionResponse(BaseModel):
    id: str
    object: str = "chat.completion"
    created: int
    model: str
    choices: list[ChatCompletionChoice]
    usage: ChatCompletionUsage


MOCK_MODEL_ID = "mock-model-v1"


@app.get("/v1/models")
async def list_models() -> dict[str, Any]:
    return {
        "object": "list",
        "data": [
            {
                "id": MOCK_MODEL_ID,
                "object": "model",
                "created": int(time.time()),
                "owned_by": "pantheon-harness",
            }
        ],
    }


@app.post("/v1/chat/completions")
async def chat_completions(req: ChatCompletionRequest) -> ChatCompletionResponse:
    # Simulate realistic latency: 100-400ms
    await asyncio.sleep(random.uniform(0.1, 0.4))

    # Simple canned response based on last user message
    last_user = next(
        (m.content for m in reversed(req.messages) if m.role == "user"),
        "",
    )
    response_text = f"[MOCK-VLLM] Echo: {last_user[:200]}"

    prompt_tokens = sum(len(m.content) // 4 for m in req.messages)
    completion_tokens = len(response_text) // 4

    return ChatCompletionResponse(
        id=f"chatcmpl-{uuid.uuid4().hex[:12]}",
        created=int(time.time()),
        model=req.model,
        choices=[
            ChatCompletionChoice(
                index=0,
                message=ChatMessage(role="assistant", content=response_text),
            )
        ],
        usage=ChatCompletionUsage(
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            total_tokens=prompt_tokens + completion_tokens,
        ),
    )


@app.get("/healthz")
async def health() -> dict[str, str]:
    return {"status": "ok"}


def serve(host: str = "0.0.0.0", port: int = 8000) -> None:
    """Entry point: `pantheon-harness --mode=mock-vllm-server` calls this."""
    import uvicorn

    uvicorn.run(app, host=host, port=port, log_level="warning")
