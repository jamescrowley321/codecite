"""Pinned, hash-checked model downloads for the spike.

    python -I models.py lock <model> [<model>...]   # pin each model to a commit, write models.lock.json
    python -I models.py verify                      # download (if needed) and check every locked file

`lock` checks each file against what the Hub itself publishes for that commit:
the LFS sha256 for large files and the git blob id for small ones. Loading
(`path`) then trusts only models.lock.json: a pinned revision, an exact file
list, and a sha256 per file. Nothing is fetched from `main` after locking.
"""
import hashlib, json, os, sys
from pathlib import Path

from huggingface_hub import HfApi, hf_hub_download

HERE = Path(__file__).parent
LOCK = HERE / "models.lock.json"
# Private to this user; /tmp/fastembed_cache was a shared, predictable path.
CACHE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "codecite-spike" / "hf"
# The files fastembed reads besides the model itself.
SIDE_FILES = ["config.json", "tokenizer.json", "tokenizer_config.json",
              "special_tokens_map.json", "preprocessor_config.json",
              # Sparse models: the inference-free query side's IDF weights.
              "idf.json"]


def _cache():
    CACHE.mkdir(parents=True, exist_ok=True)
    CACHE.chmod(0o700)
    return CACHE


def _sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def _git_blob_id(p):
    data = Path(p).read_bytes()
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def _source(model):
    from fastembed import LateInteractionTextEmbedding, SparseTextEmbedding, TextEmbedding
    from fastembed.rerank.cross_encoder import TextCrossEncoder
    for cls in (TextEmbedding, TextCrossEncoder, SparseTextEmbedding, LateInteractionTextEmbedding):
        try:
            supported = cls._list_supported_models()
        except AttributeError as e:
            raise SystemExit(
                f"fastembed private API changed ({e}). "
                f"This script is pinned to fastembed==0.9.0; regenerate the lockfile with care."
            ) from e
        for m in supported:
            if m.model == model:
                return m.sources.hf, m.model_file
    raise SystemExit(f"unknown fastembed model: {model}")


def lock(models):
    api = HfApi()
    out = json.loads(LOCK.read_text()) if LOCK.exists() else {}
    for model in models:
        repo, model_file = _source(model)
        info = api.model_info(repo, revision="main", files_metadata=True)
        siblings = {s.rfilename: s for s in info.siblings}
        wanted = [f for f in SIDE_FILES if f in siblings] + [model_file]
        # An external-data ONNX model keeps its weights next to it.
        if model_file + "_data" in siblings:
            wanted.append(model_file + "_data")
        files = {}
        for name in wanted:
            s = siblings[name]
            p = hf_hub_download(repo, name, revision=info.sha, cache_dir=_cache())
            got = _sha256(p)
            if s.lfs:
                if got != s.lfs.sha256:
                    raise SystemExit(f"{repo}/{name}: sha256 {got} != hub {s.lfs.sha256}")
            elif _git_blob_id(p) != s.blob_id:
                raise SystemExit(f"{repo}/{name}: git blob id != hub {s.blob_id}")
            files[name] = got
        out[model] = {"repo": repo, "revision": info.sha, "files": files}
        print(f"locked {model} -> {repo}@{info.sha} ({len(files)} files)")
    LOCK.write_text(json.dumps(out, indent=1, sort_keys=True) + "\n")


def path(model):
    """The snapshot directory for `model`, every file checked against the lock."""
    entry = json.loads(LOCK.read_text()).get(model)
    if entry is None:
        raise SystemExit(f"{model} is not in {LOCK.name}; run `models.py lock {model}`")
    snap = None
    for name, want in entry["files"].items():
        p = hf_hub_download(entry["repo"], name, revision=entry["revision"], cache_dir=_cache())
        if (got := _sha256(p)) != want:
            raise SystemExit(f"{model}: {name} sha256 {got} != locked {want}")
        snap = Path(p).parents[len(Path(name).parts) - 1]
    return str(snap)


if __name__ == "__main__":
    cmd, *rest = sys.argv[1:] or ["verify"]
    if cmd == "lock":
        lock(rest)
    elif cmd == "verify":
        for m in json.loads(LOCK.read_text()):
            print(m, "ok", path(m))
    else:
        raise SystemExit(__doc__)
