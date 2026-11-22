import json
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

import seed


class PublicSeedTests(unittest.TestCase):
    def test_only_public_book_and_index_aligned_weighted_catalog(self):
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch) / "data"
            receipt = seed.create_seed(directory)
            self.assertEqual(list(directory.iterdir()), [directory / "library.sqlite"])
            with closing(sqlite3.connect(f"file:{directory}/library.sqlite?mode=ro", uri=True)) as database:
                self.assertEqual(database.execute("SELECT id,title,source,total_chapters,cached_chapters,on_shelf FROM books").fetchall(),
                                 [(seed.BOOK, seed.TITLE, "local", 1863, 1, 1)])
                self.assertEqual(database.execute("SELECT book_id,chapter_index,char_offset FROM positions").fetchall(), [(seed.BOOK, 0, 0)])
                self.assertFalse(database.execute("SELECT 1 FROM download_tasks").fetchone())
                settings = dict(database.execute("SELECT key,value FROM settings"))
                self.assertEqual(set(settings), {f"catalog.remote:{seed.BOOK}", f"reader.preferences:{seed.BOOK}"})
                self.assertEqual(json.loads(settings[f"reader.preferences:{seed.BOOK}"])["refresh_interval"], 6)
                chapters = database.execute("SELECT chapter_index,source_id,title,body,content_status,content_account_key FROM chapters ORDER BY chapter_index").fetchall()
                catalog = json.loads(settings[f"catalog.remote:{seed.BOOK}"])
                self.assertEqual(len(chapters), 1863)
                self.assertEqual(len(catalog), len(chapters))
                for index, (stored, remote) in enumerate(zip(chapters, catalog)):
                    self.assertEqual(stored[:3], (index, remote["uid"], remote["title"]))
                    self.assertEqual(remote["index"], index)
                    self.assertEqual(remote["resource_url"], "")
                    self.assertEqual(stored[4:], ("unknown" if index else "full", ""))
                    self.assertEqual(remote["word_count"], index + 1 if index else len(stored[3]))
                    self.assertEqual(stored[3] is None, index > 0)
                body = chapters[0][3]
                self.assertEqual(receipt["body_scalars"], len(body))
                self.assertTrue(body.startswith("Section 0001."))
                self.assertEqual(len(body.splitlines()), 1600)
                self.assertTrue(all(line.startswith(f"Section {index:04}.") for index, line in enumerate(body.splitlines(), 1)))
                self.assertFalse(body.isascii())
            with self.assertRaises(FileExistsError):
                seed.create_seed(directory)


if __name__ == "__main__":
    unittest.main()
