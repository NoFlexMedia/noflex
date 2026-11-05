#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
#
# Measures an embedding export on BEIR SciFact: speed, and nDCG@10 over the full
# corpus. Run it before changing packaging/model.toml. Exits rather than report a
# provider ONNX Runtime didn't actually use, which it otherwise falls back from
# quietly. Needs onnxruntime, tokenizers and numpy. Usage: embeddings.py MODEL_DIR
# SCIFACT_DIR PROVIDER [THREADS]

import csv
import json
import sys
import time

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

model_dir, scifact, provider = sys.argv[1:4]
threads = int(sys.argv[4]) if len(sys.argv) > 4 else 4

options = ort.SessionOptions()
options.add_session_config_entry("session.disable_quant_qdq", "1")
options.intra_op_num_threads = threads
session = ort.InferenceSession(f"{model_dir}/model_text.onnx", options, providers=[provider])
if session.get_providers()[0] != provider:
    sys.exit(f"wanted {provider}, got {session.get_providers()[0]}")

tokenizer = Tokenizer.from_file(f"{model_dir}/tokenizer.json")
tokenizer.no_padding()
tokenizer.enable_truncation(256)

documents = [json.loads(line) for line in open(f"{scifact}/corpus.jsonl")]
queries = {row["_id"]: row["text"] for row in map(json.loads, open(f"{scifact}/queries.jsonl"))}
relevant = {}
for row in csv.DictReader(open(f"{scifact}/qrels/test.tsv"), delimiter="\t"):
    relevant.setdefault(row["query-id"], {})[row["corpus-id"]] = int(row["score"])
query_ids = list(relevant)


def embed(texts, batch_size):
    encoded = [e.ids for e in tokenizer.encode_batch(texts)]
    order = sorted(range(len(encoded)), key=lambda i: len(encoded[i]))
    out = np.zeros((len(encoded), 768), np.float32)
    for start in range(0, len(order), batch_size):
        rows = order[start : start + batch_size]
        width = max(len(encoded[i]) for i in rows)
        ids = np.zeros((len(rows), width), np.int64)
        mask = np.zeros((len(rows), width), np.int64)
        for row, i in enumerate(rows):
            ids[row, : len(encoded[i])] = encoded[i]
            mask[row, : len(encoded[i])] = 1
        out[rows] = session.run(["sentence_embedding"], {"input_ids": ids, "attention_mask": mask})[0]
    return out


def unit(vectors):
    return vectors / np.maximum(np.linalg.norm(vectors, axis=-1, keepdims=True), 1e-12)


def ndcg(doc_vectors, query_vectors, dimensions):
    scores = unit(query_vectors[:, :dimensions]) @ unit(doc_vectors[:, :dimensions]).T
    total = 0.0
    for row, query_id in enumerate(query_ids):
        gains = [relevant[query_id].get(documents[i]["_id"], 0) for i in np.argsort(-scores[row])[:10]]
        ideal = sorted(relevant[query_id].values(), reverse=True)[:10]
        total += sum(g / np.log2(k + 2) for k, g in enumerate(gains)) / sum(
            g / np.log2(k + 2) for k, g in enumerate(ideal)
        )
    return round(total / len(query_ids), 4)


query_texts = [f"task: search result | query: {queries[i]}" for i in query_ids]
document_texts = [f"title: {d['title'] or 'none'} | text: {d['text']}" for d in documents]

for text in query_texts[:3]:
    embed([text], 1)
latencies = []
for text in query_texts[:40]:
    started = time.perf_counter()
    embed([text], 1)
    latencies.append(time.perf_counter() - started)

started = time.perf_counter()
document_vectors = embed(document_texts, 8)
elapsed = time.perf_counter() - started
query_vectors = embed(query_texts, 8)

print(
    json.dumps(
        {
            "provider": session.get_providers()[0],
            "threads": threads,
            "query_ms_p50": round(float(np.median(latencies)) * 1e3, 1),
            "docs_per_s": round(len(document_texts) / elapsed, 1),
            "ndcg10": {d: ndcg(document_vectors, query_vectors, d) for d in (768, 512, 256)},
        }
    )
)
