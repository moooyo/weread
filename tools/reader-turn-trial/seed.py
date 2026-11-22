"""Create a fresh public Reader data directory on the worker, never on the Kindle."""

import argparse
import json
import sqlite3
from contextlib import closing
from pathlib import Path


BOOK = "pointer-turn-fixture"
TITLE = "Public Pointer Turn Fixture"
CHAPTERS = 1863
SECTIONS = 1600
PROFILE = "scribe"
FIRMWARE = "5.19.5"
STORE = Path(__file__).resolve().parents[2] / "src" / "store.rs"


def current_schema():
    function = STORE.read_text(encoding="utf-8").split("fn initialize_schema(", 1)[1]
    start = 'transaction.execute_batch("'
    sql = function.split(start, 1)[1].split('\n    ")?;', 1)[0]
    if '"' in sql or "CREATE TABLE IF NOT EXISTS chapters (" not in sql:
        raise ValueError("The current Store schema is not a single recognized SQL literal")
    return sql


def chapter_uid(index):
    return f"public-chapter-{index:04}"


def chapter_title(index):
    return f"Public Reader Chapter {index:04}"


def public_body():
    return "\n".join(
        f"Section {section:04}. Public reading sample {section:04}: π · Ω · →."
        for section in range(1, SECTIONS + 1)
    )


def create_seed(directory):
    schema = current_schema()
    body = public_body()
    catalog = [
        {"uid": chapter_uid(index), "title": chapter_title(index), "index": index,
         "word_count": len(body) if index == 0 else index + 1, "resource_url": ""}
        for index in range(CHAPTERS)
    ]
    reader = {"font": "song", "font_size": 24, "line_spacing": 1, "margin": 1,
              "weight": 400, "indent": 2, "gap": 0, "justify": True,
              "tap_mode": "default", "refresh_interval": 6, "book_only": True}
    directory.mkdir(mode=0o700, exist_ok=False)
    database = directory / "library.sqlite"
    try:
        with closing(sqlite3.connect(database)) as connection:
            connection.execute("PRAGMA foreign_keys=ON")
            connection.executescript(schema)
            with connection:
                connection.execute(
                    "INSERT INTO books(id,title,author,category,intro,cover_url,source,"
                    "total_chapters,cached_chapters,on_shelf) VALUES (?,?,'Public Fixture',"
                    "'','','','local',?,1,1)", (BOOK, TITLE, CHAPTERS))
                connection.executemany(
                    "INSERT INTO chapters(book_id,chapter_index,source_id,title,body,content_status) "
                    "VALUES (?,?,?,?,?,?)",
                    ((BOOK, row["index"], row["uid"], row["title"],
                      body if row["index"] == 0 else None, "full" if row["index"] == 0 else "unknown")
                     for row in catalog))
                connection.execute(
                    "INSERT INTO positions(book_id,chapter_index,char_offset,updated_at,device) "
                    "VALUES (?,0,0,0,'Public fixture')", (BOOK,))
                connection.executemany("INSERT INTO settings(key,value) VALUES (?,?)", [
                    (f"catalog.remote:{BOOK}", json.dumps(catalog, separators=(",", ":"))),
                    (f"reader.preferences:{BOOK}", json.dumps(reader, separators=(",", ":"))),
                ])
    except BaseException:
        database.unlink(missing_ok=True)
        directory.rmdir()
        raise
    return {"book_id": BOOK, "chapters": CHAPTERS, "sections": SECTIONS,
            "chapter": 0, "initial_page": 1, "initial_anchor": 0, "body_scalars": len(body),
            "profile": PROFILE, "firmware": FIRMWARE, "refresh_interval": 6}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="new data directory under an existing worker scratch directory")
    args = parser.parse_args()
    print(json.dumps(create_seed(args.directory), sort_keys=True))


if __name__ == "__main__":
    main()
