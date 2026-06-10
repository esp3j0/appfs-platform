#!/usr/bin/env python3
"""Probe whether a Z.ai Anthropic-compatible endpoint accepts tool_result.is_error.

The script sends two minimal Anthropic Messages API requests:
1. A synthetic assistant tool_use followed by a user tool_result with is_error=true.
2. The same request with the is_error field omitted.

If request 1 fails while request 2 succeeds, the upstream likely does not support
Anthropic's error tool_result shape even though it accepts normal tool results.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any


DEFAULT_BASE_URL = "https://api.z.ai/api/anthropic"
DEFAULT_MODEL = "claude-opus-4-6"
ANTHROPIC_VERSION = "2023-06-01"


@dataclass
class HttpResult:
    name: str
    ok: bool
    status: int | None
    request_id: str | None
    body: str
    elapsed_ms: int


def env_first(*names: str) -> str | None:
    for name in names:
        value = os.environ.get(name)
        if value:
            return value
    return None


def messages_url(base_url: str) -> str:
    base = base_url.rstrip("/")
    if base.endswith("/v1/messages"):
        return base
    return f"{base}/v1/messages"


def build_payload(model: str, include_is_error: bool) -> dict[str, Any]:
    tool_result: dict[str, Any] = {
        "type": "tool_result",
        "tool_use_id": "toolu_zai_is_error_probe_1",
        "content": [
            {
                "type": "text",
                "text": "Synthetic tool failure: file not found. Please acknowledge the failure.",
            }
        ],
    }
    if include_is_error:
        tool_result["is_error"] = True

    return {
        "model": model,
        "max_tokens": 128,
        "stream": False,
        "tools": [
            {
                "name": "debug_echo",
                "description": "A synthetic tool used only for protocol compatibility testing.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "string"},
                    },
                    "required": ["value"],
                },
            }
        ],
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": (
                            "Protocol validation test. Continue from the supplied tool "
                            "result and answer in one short sentence."
                        ),
                    }
                ],
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_zai_is_error_probe_1",
                        "name": "debug_echo",
                        "input": {"value": "hello"},
                    }
                ],
            },
            {
                "role": "user",
                "content": [tool_result],
            },
        ],
    }


def build_headers(api_key: str, auth_mode: str) -> dict[str, str]:
    headers = {
        "content-type": "application/json",
        "anthropic-version": ANTHROPIC_VERSION,
    }
    if auth_mode in {"bearer", "both"}:
        headers["authorization"] = f"Bearer {api_key}"
    if auth_mode in {"x-api-key", "both"}:
        headers["x-api-key"] = api_key
    return headers


def post_json(
    *,
    name: str,
    url: str,
    payload: dict[str, Any],
    headers: dict[str, str],
    timeout: float,
) -> HttpResult:
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(url, data=data, headers=headers, method="POST")
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8", errors="replace")
            elapsed_ms = int((time.perf_counter() - started) * 1000)
            return HttpResult(
                name=name,
                ok=200 <= response.status < 300,
                status=response.status,
                request_id=response.headers.get("request-id")
                or response.headers.get("x-request-id"),
                body=body,
                elapsed_ms=elapsed_ms,
            )
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        elapsed_ms = int((time.perf_counter() - started) * 1000)
        return HttpResult(
            name=name,
            ok=False,
            status=error.code,
            request_id=error.headers.get("request-id") or error.headers.get("x-request-id"),
            body=body,
            elapsed_ms=elapsed_ms,
        )
    except urllib.error.URLError as error:
        elapsed_ms = int((time.perf_counter() - started) * 1000)
        return HttpResult(
            name=name,
            ok=False,
            status=None,
            request_id=None,
            body=str(error),
            elapsed_ms=elapsed_ms,
        )


def print_result(result: HttpResult, body_chars: int) -> None:
    status = result.status if result.status is not None else "network-error"
    verdict = "OK" if result.ok else "FAIL"
    print(f"\n== {result.name}: {verdict} status={status} elapsed={result.elapsed_ms}ms")
    if result.request_id:
        print(f"request-id: {result.request_id}")
    body = result.body.strip()
    if body:
        if len(body) > body_chars:
            body = f"{body[:body_chars]}...<truncated>"
        print(body)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-url",
        default=env_first("ZAI_ANTHROPIC_BASE_URL", "ANTHROPIC_BASE_URL", "ZAI_BASE_URL")
        or DEFAULT_BASE_URL,
        help=f"Anthropic-compatible base URL. Default: {DEFAULT_BASE_URL}",
    )
    parser.add_argument(
        "--model",
        default=env_first("ZAI_MODEL", "ANTHROPIC_MODEL") or DEFAULT_MODEL,
        help=f"Model name to send. Default: {DEFAULT_MODEL}",
    )
    parser.add_argument(
        "--api-key",
        default=env_first("ZAI_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"),
        help="API key. Prefer env ZAI_API_KEY, ANTHROPIC_AUTH_TOKEN, or ANTHROPIC_API_KEY.",
    )
    parser.add_argument(
        "--auth-mode",
        choices=["bearer", "x-api-key", "both"],
        default=os.environ.get("ZAI_AUTH_MODE", "bearer"),
        help="Auth header style. Default: bearer",
    )
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--body-chars", type=int, default=4000)
    parser.add_argument(
        "--dump-payload",
        action="store_true",
        help="Print the request payloads before sending. Does not print credentials.",
    )
    args = parser.parse_args()

    if not args.api_key:
        print(
            "Missing API key. Set ZAI_API_KEY or ANTHROPIC_AUTH_TOKEN, "
            "or pass --api-key.",
            file=sys.stderr,
        )
        return 64

    url = messages_url(args.base_url)
    headers = build_headers(args.api_key, args.auth_mode)

    print(f"URL: {url}")
    print(f"model: {args.model}")
    print(f"auth-mode: {args.auth_mode}")

    payload_with_error = build_payload(args.model, include_is_error=True)
    payload_without_error = build_payload(args.model, include_is_error=False)

    if args.dump_payload:
        print("\nPayload with is_error=true:")
        print(json.dumps(payload_with_error, ensure_ascii=False, indent=2))
        print("\nPayload without is_error:")
        print(json.dumps(payload_without_error, ensure_ascii=False, indent=2))

    with_error = post_json(
        name="tool_result with is_error=true",
        url=url,
        payload=payload_with_error,
        headers=headers,
        timeout=args.timeout,
    )
    print_result(with_error, args.body_chars)

    without_error = post_json(
        name="tool_result without is_error",
        url=url,
        payload=payload_without_error,
        headers=headers,
        timeout=args.timeout,
    )
    print_result(without_error, args.body_chars)

    print("\n== verdict")
    if with_error.ok:
        print("Upstream accepted tool_result.is_error=true.")
        return 0
    if without_error.ok:
        print(
            "Upstream rejected is_error=true but accepted the same tool_result "
            "when is_error was omitted. Use a provider compatibility shim."
        )
        return 2

    print(
        "Both requests failed. Check auth/model/base URL first; this does not "
        "isolate is_error compatibility yet."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
