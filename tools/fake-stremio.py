#!/usr/bin/env python3
"""A stand-in for Stremio, to run anchor without an account.

Serves the Stremio API (sign in with demo@example.com and demo), a
metadata addon with movie and series catalogs (genres, paging, search) and
a stream addon. Streams play a generated test picture in mpv, or the files
given with --media. The library lives in memory: what anchor writes back
shows on the next sync.

    python3 tools/fake-stremio.py [--port 8099] [--posters DIR] [--media FILE ...]
    ANCHOR_API_URL=http://127.0.0.1:8099/api/ cargo run --release

--posters takes a directory of pictures: poster-*.jpg are used as posters
and still-*.jpg as backdrops and episode stills, in name order.
"""

import argparse
import json
import os
import re
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

EMAIL, PASSWORD, KEY = "demo@example.com", "demo", "fake-auth-key"

GENRES = ["Action", "Adventure", "Animation", "Comedy", "Crime", "Drama", "Family", "Mystery"]
MOVIES = [
    ("The Long Burrow", 2025, 124, "7.4", "Adventure", "Family"),
    ("Red Fury", 2025, 98, "6.9", "Action", "Comedy"),
    ("Die Lichtung", 2025, 112, "7.2", "Drama", "Mystery"),
    ("Apples in August", 2026, 91, "6.6", "Family", "Comedy"),
    ("Stakes", 2024, 105, "7.0", "Crime", "Drama"),
    ("Glide", 2025, 101, "7.7", "Adventure", "Animation"),
    ("Second Thoughts", 2026, 94, "6.4", "Comedy", "Drama"),
    ("Small Hours", 2024, 117, "7.1", "Crime", "Mystery"),
    ("The Pact", 2023, 89, "6.2", "Comedy", "Family"),
    ("Far Field", 2025, 126, "7.0", "Drama", "Adventure"),
    ("Le Terrier", 2024, 99, "7.3", "Mystery", "Animation"),
    ("Shade", 2023, 108, "6.8", "Drama", "Mystery"),
    ("The Gang of Three", 2025, 96, "7.5", "Comedy", "Crime"),
    ("Big Feelings", 2026, 88, "6.3", "Animation", "Family"),
    ("Nutcase", 2024, 93, "8.3", "Animation", "Comedy"),
    ("Undergrowth", 2025, 103, "8.6", "Mystery", "Drama"),
    ("Sharp Ends", 2024, 110, "7.5", "Action", "Crime"),
    ("Burrowers", 2023, 97, "6.1", "Adventure", "Action"),
    ("Under the Burrow", 2019, 102, "6.8", "Family", "Adventure"),
    ("Thistle and Bramble", 2022, 115, "7.9", "Drama", "Family"),
    ("Brook Crossing", 2021, 92, "6.7", "Adventure", "Family"),
    ("Last Light on the Ridge", 2020, 131, "8.1", "Drama", "Action"),
    ("Hollow Creek", 2018, 99, "6.5", "Mystery", "Crime"),
    ("Twelve Winters", 2017, 140, "8.0", "Drama", "Adventure"),
    ("The Quiet Glen", 2016, 87, "6.9", "Family", "Animation"),
    ("Fieldnotes", 2015, 95, "7.2", "Comedy", "Drama"),
    ("Orchard at Dusk", 2019, 106, "7.6", "Drama", "Mystery"),
    ("Nine Acorns", 2021, 84, "6.0", "Animation", "Comedy"),
    ("Velvet Hollow", 2022, 119, "7.8", "Crime", "Drama"),
    ("Under the Old Oak", 2020, 101, "7.1", "Family", "Adventure"),
]
SERIES = [
    ("Small Thieves", 2024, "7.8", "Comedy", "Family", [8, 10, 6]),
    ("Down the Burrow", 2025, "7.9", "Adventure", "Drama", [6, 8]),
    ("The Warren", 2022, "8.2", "Drama", "Mystery", [10, 10, 10, 1]),
    ("Meadow Hall", 2026, "8.0", "Drama", "Family", [9]),
    ("Rookery", 2023, "7.3", "Crime", "Comedy", [6, 6]),
    ("High Pasture", 2025, "6.8", "Adventure", "Drama", [8]),
]
CAST = [
    ("Teo Larsen", "Bramble"), ("Margit Holm", "Wren"), ("Sami Okafor", "Frank"), ("Lena Ruiz", "Rinky"),
    ("Odile Brandt", "Gimera"), ("Jonas Pihl", "Old Thistle"), ("Kofi Mensah", "Burdock"),
    ("Mira Solberg", "Twig"), ("Ruth Aalto", "Hazel"), ("Ines Varga", "Narrator"),
]
PLOTS = [
    "When the spring floods reach the old warren, a stubborn rabbit leads three unlikely neighbours on a long dig to higher ground.",
    "Three woodland rodents run the most ambitious heist crew in the valley. Their only problem is the rabbit next door.",
    "A summer of quiet afternoons turns strange when the orchard's oldest tree starts dropping something other than apples.",
    "Two rivals, one meadow, and a very long winter that neither of them planned for.",
]


def slug(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")


class Content:
    def __init__(self, base, posters):
        self.base = base
        pictures = sorted(os.listdir(posters)) if posters else []
        self.posters = [f for f in pictures if f.startswith("poster")]
        self.stills = [f for f in pictures if f.startswith("still")]
        self.posters_dir = posters
        self.movies = [self.movie(i, m) for i, m in enumerate(MOVIES)]
        self.series = [self.show(i, s) for i, s in enumerate(SERIES)]

    def picture(self, kind, i):
        files = self.posters if kind == "poster" else self.stills
        return f"{self.base}/art/{files[i % len(files)]}" if files else None

    def movie(self, i, m):
        name, year, minutes, rating, *genres = m
        return {
            "id": f"tt9{i:06d}", "type": "movie", "name": name, "poster": self.picture("poster", i),
            "background": self.picture("still", i), "description": PLOTS[i % len(PLOTS)],
            "releaseInfo": str(year), "runtime": f"{minutes} min", "genres": genres, "imdbRating": rating,
            "links": [{"name": "Ana Dimas", "category": "Directors", "url": "stremio:///x"},
                      {"name": "Ana Dimas", "category": "Writers", "url": "stremio:///x"},
                      {"name": "Ines Varga", "category": "Writers", "url": "stremio:///x"},
                      {"name": "Teo Larsen", "category": "Cast", "url": "stremio:///x"},
                      {"name": "Margit Holm", "category": "Cast", "url": "stremio:///x"}],
            "released": f"{year}-03-14T00:00:00.000Z", "country": "Netherlands",
            # Cinemeta sends awards; AIOMetadata the cast with pictures and
            # the certification.
            **({"awards": "2 wins and 5 nominations"} if i % 2 == 0 else {}),
            "app_extras": {
                "certification": "PG",
                "cast": [{"name": name, "character": part,
                          "photo": None if k == 5 else self.picture("poster", i + k)}
                         for k, (name, part) in enumerate(CAST)],
            },
            "behaviorHints": {"defaultVideoId": f"tt9{i:06d}", "hasScheduledVideos": False},
            "videos": [],
        }

    def show(self, i, s):
        name, year, rating, g1, g2, seasons = s
        sid = f"tt8{i:06d}"
        videos, day = [], 0
        now = time.time()
        for season, count in enumerate(seasons, start=1):
            for episode in range(1, count + 1):
                day += 7
                # The last episode of the last season comes out next week.
                upcoming = season == len(seasons) and episode == count and i == 0
                released = now + 7 * 86400 if upcoming else now - (400 - day) * 86400
                videos.append({
                    "id": f"{sid}:{season}:{episode}", "season": season, "episode": episode,
                    "name": f"Episode {episode}" if episode > 3 else ["Nuts About You", "The Orchard Job", "Tree House Rules"][episode - 1],
                    "overview": PLOTS[(season + episode) % len(PLOTS)],
                    "released": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(released)),
                    "thumbnail": self.picture("still", season * 10 + episode),
                })
        return {
            "id": sid, "type": "series", "name": name, "poster": self.picture("poster", i + 7),
            "background": self.picture("still", i + 3), "description": PLOTS[i % len(PLOTS)],
            "releaseInfo": f"{year}-", "genres": [g1, g2], "imdbRating": rating,
            "links": [{"name": "Ana Dimas", "category": "Cast", "url": "stremio:///x"}],
            "behaviorHints": {"defaultVideoId": None, "hasScheduledVideos": i == 0},
            "videos": videos,
        }

    def manifest_meta(self):
        def catalog(kind, cid, name, search=False):
            extra = [{"name": "genre", "options": GENRES}, {"name": "skip"}]
            if search:
                extra.append({"name": "search"})
            return {"type": kind, "id": cid, "name": name, "extra": extra}
        return {
            "id": "fake.metadata", "version": "1.0.0", "name": "Fake Metadata",
            "description": "Catalogs and metadata for anchor's tests",
            "types": ["movie", "series"], "idPrefixes": ["tt"], "resources": ["catalog", "meta"],
            "catalogs": [
                catalog("movie", "trending", "Trending", search=True), catalog("movie", "top", "Top rated"),
                catalog("movie", "new", "New releases"), catalog("series", "trending", "Trending", search=True),
                catalog("series", "top", "Top rated"),
            ],
        }

    def manifest_streams(self):
        return {"id": "fake.streams", "version": "1.0.0", "name": "Fake Streams",
                "types": ["movie", "series"], "idPrefixes": ["tt"],
                "resources": [{"name": "stream", "types": ["movie", "series"], "idPrefixes": ["tt"]}]}

    def catalog(self, kind, cid, extra):
        items = self.movies if kind == "movie" else self.series
        if cid == "top":
            items = sorted(items, key=lambda m: -float(m["imdbRating"]))
        elif cid == "new":
            items = sorted(items, key=lambda m: m["releaseInfo"], reverse=True)
        if "genre" in extra:
            items = [m for m in items if extra["genre"] in m["genres"]]
        if "search" in extra:
            words = extra["search"].lower().split()
            items = [m for m in items if all(w in m["name"].lower() for w in words)]
        skip = int(extra.get("skip", 0))
        page = items[skip:skip + 20]
        return {"metas": [{k: v for k, v in m.items() if k != "videos"} for m in page]}

    def meta(self, mid):
        return next((m for m in self.movies + self.series if m["id"] == mid), None)


class Fake:
    def __init__(self, base, posters, media):
        self.content = Content(base, posters)
        self.media = media
        self.base = base
        self.library = {item["_id"]: item for item in self.seed()}
        self.lock = threading.Lock()

    def seed(self):
        """Two titles left unfinished: an episode and a movie."""
        def item(meta, video, offset, duration, minutes_ago):
            when = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - minutes_ago * 60))
            return {
                "_id": meta["id"], "name": meta["name"], "type": meta["type"], "poster": meta["poster"],
                "posterShape": "poster", "removed": False, "temp": False, "_ctime": when, "_mtime": when,
                "state": {"lastWatched": when, "timeWatched": offset, "timeOffset": offset,
                          "overallTimeWatched": offset, "timesWatched": 0, "flaggedWatched": 0,
                          "duration": duration, "video_id": video, "watched": None, "noNotif": False},
                "behaviorHints": {"defaultVideoId": meta["behaviorHints"]["defaultVideoId"],
                                  "featuredVideoId": None, "hasScheduledVideos": False},
            }
        show, movie = self.content.series[0], self.content.movies[5]
        return [
            item(show, f"{show['id']}:2:4", 30 * 60000, 48 * 60000, 10),
            item(movie, movie["id"], 40 * 60000, 101 * 60000, 60 * 24),
        ]

    def streams(self, vid):
        files = [f"{self.base}/media/{i}/{os.path.basename(f)}" for i, f in enumerate(self.media)]
        if not files:
            files = ["av://lavfi:testsrc2=duration=3000:size=1920x1080:rate=24"]
        release = vid.replace(":", ".")
        out = []
        for i, (tag, quality, kind) in enumerate([
            ("[TB⚡]", "2160p", "WEB-DL · HEVC · DV · HDR10"),
            ("[RD⚡]", "2160p", "REMUX · HEVC · DV · HDR10"),
            ("[TB⚡]", "1080p", "WEB-DL · AVC"),
            ("[RD⚡]", "720p", "WEBRip · AVC"),
        ]):
            out.append({
                "name": f"{tag}\n{quality}", "url": files[i % len(files)],
                "description": f"{release}.{quality}.FAKE\n🎞️ {kind}\n🔊 DD+ 5.1 · 🌐 English\n💾 {18 - i * 4}.2 GB · Fake",
                "behaviorHints": {"bingeGroup": f"fake|{quality}", "filename": f"{release}.{quality}.mkv"},
            })
        return {"streams": out}


def handler_for(fake):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            pass

        def send_json(self, value, status=200):
            body = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(length) or b"{}")
            method = self.path.rsplit("/", 1)[-1]
            if method == "login":
                if body.get("email") == EMAIL and body.get("password") == PASSWORD:
                    return self.send_json({"result": {"authKey": KEY, "user": {"_id": "fake-user", "email": EMAIL}}})
                if body.get("email") != EMAIL:
                    return self.send_json({"error": {"code": 2, "message": "User not found"}})
                return self.send_json({"error": {"code": 3, "message": "Wrong passphrase"}})
            if body.get("authKey") != KEY:
                return self.send_json({"error": {"code": 1, "message": "Session does not exist"}})
            if method == "logout":
                return self.send_json({"result": {"success": True}})
            if method == "addonCollectionGet":
                addons = [
                    {"transportUrl": f"{fake.base}/meta/manifest.json", "manifest": fake.content.manifest_meta()},
                    {"transportUrl": f"{fake.base}/streams/manifest.json", "manifest": fake.content.manifest_streams()},
                ]
                return self.send_json({"result": {"addons": addons, "lastModified": "2026-10-01T00:00:00Z"}})
            if method == "datastoreGet":
                with fake.lock:
                    return self.send_json({"result": list(fake.library.values())})
            if method == "datastorePut":
                with fake.lock:
                    for item in body.get("changes", []):
                        fake.library[item["_id"]] = item
                return self.send_json({"result": {"success": True}})
            self.send_json({"error": {"code": 0, "message": "Unknown method"}})

        def do_GET(self):
            path = urllib.parse.unquote(self.path.split("?", 1)[0])
            parts = path.strip("/").split("/")
            if parts[0] == "art" and fake.content.posters_dir:
                return self.send_file(os.path.join(fake.content.posters_dir, os.path.basename(path)))
            if parts[0] == "media" and len(parts) > 1 and parts[1].isdigit():
                return self.send_file(fake.media[int(parts[1])])
            if path == "/meta/manifest.json":
                return self.send_json(fake.content.manifest_meta())
            if path == "/streams/manifest.json":
                return self.send_json(fake.content.manifest_streams())
            m = re.match(r"^/(meta|streams)/(\w+)/(\w+)/([^/]+?)(?:/([^/]+))?\.json$", path)
            if not m:
                return self.send_json({"err": "not found"}, 404)
            addon, resource, kind, rid, extra = m.groups()
            extra = dict(urllib.parse.parse_qsl(extra or ""))
            time.sleep(0.3)
            if addon == "meta" and resource == "catalog":
                return self.send_json(fake.content.catalog(kind, rid, extra))
            if addon == "meta" and resource == "meta":
                meta = fake.content.meta(rid)
                return self.send_json({"meta": meta} if meta else {"err": "not found"}, 200 if meta else 404)
            if addon == "streams" and resource == "stream":
                return self.send_json(fake.streams(rid))
            self.send_json({"err": "not found"}, 404)

        def send_file(self, name):
            try:
                size = os.path.getsize(name)
            except OSError:
                return self.send_json({"err": "not found"}, 404)
            start, end = 0, size - 1
            ranged = re.match(r"bytes=(\d+)-(\d*)", self.headers.get("Range", ""))
            if ranged:
                start = int(ranged.group(1))
                end = int(ranged.group(2)) if ranged.group(2) else size - 1
            self.send_response(206 if ranged else 200)
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            if ranged:
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.end_headers()
            with open(name, "rb") as f:
                f.seek(start)
                left = end - start + 1
                while left > 0:
                    chunk = f.read(min(left, 1 << 16))
                    if not chunk:
                        break
                    try:
                        self.wfile.write(chunk)
                    except (BrokenPipeError, ConnectionResetError):
                        return
                    left -= len(chunk)

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--port", type=int, default=8099)
    parser.add_argument("--posters", help="a directory of poster-*.jpg and still-*.jpg")
    parser.add_argument("--media", nargs="*", default=[], help="video files the streams play")
    args = parser.parse_args()
    base = f"http://127.0.0.1:{args.port}"
    fake = Fake(base, args.posters, args.media)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler_for(fake))
    print(f"Fake Stremio at {base}/api/ (sign in with {EMAIL} / {PASSWORD})")
    print(f"Run anchor with ANCHOR_API_URL={base}/api/")
    server.serve_forever()


if __name__ == "__main__":
    main()
