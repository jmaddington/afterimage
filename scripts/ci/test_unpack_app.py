"""python3 scripts/ci/test_unpack_app.py"""
import os, stat, sys, tempfile, unittest, zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from unpack_app import unpack

APP = "A.app"


def make(entries):
    f = tempfile.NamedTemporaryFile(suffix=".zip", delete=False).name
    with zipfile.ZipFile(f, "w") as z:
        for name, data, mode in entries:
            i = zipfile.ZipInfo(name)
            i.create_system = 3
            i.external_attr = mode << 16
            z.writestr(i, data)
    return f


R, X, D, L = stat.S_IFREG | 0o644, stat.S_IFREG | 0o755, stat.S_IFDIR | 0o755, stat.S_IFLNK | 0o777


class T(unittest.TestCase):
    def test_an_app_unpacks_with_its_modes(self):
        out = tempfile.mkdtemp()
        unpack(make([("A.app/", "", D), ("A.app/Contents/MacOS/A", "bin", X), ("A.app/Contents/Info.plist", "p", R)]), APP, out)
        p = os.path.join(out, "A.app/Contents/MacOS/A")
        self.assertEqual(open(p).read(), "bin")
        self.assertTrue(os.stat(p).st_mode & 0o100)
        self.assertFalse(os.stat(os.path.join(out, "A.app/Contents/Info.plist")).st_mode & 0o111)

    def test_anything_else_is_refused_and_nothing_is_written(self):
        ok = ("A.app/x", "1", R)
        for bad in [("../x", "", R), ("A.app/../../x", "", R), ("/etc/x", "", R), ("B.app/x", "", R), ("scripts/sign.sh", "", R), ("A.app/./x", "", R), ("A.app//x", "", R),
                    ("A.app/l", "/etc/passwd", L), ("A.app/f", "", stat.S_IFIFO | 0o644), ("A.app/d", "", D), ("A.app\\x", "", R), ("A.app/X", "", R), ("A.app/x", "", R)]:
            out = tempfile.mkdtemp()
            with self.assertRaises(ValueError, msg=bad[0]):
                unpack(make([ok, bad]), APP, out)
            self.assertEqual(os.listdir(out), [], bad[0])

    def test_empty_full_and_broken(self):
        for f in [make([])]:
            with self.assertRaises(ValueError):
                unpack(f, APP, tempfile.mkdtemp())
        full = tempfile.mkdtemp()
        open(os.path.join(full, "a"), "w").close()
        with self.assertRaises(ValueError):
            unpack(make([("A.app/x", "1", R)]), APP, full)
        t = tempfile.NamedTemporaryFile(delete=False)
        t.write(b"not a zip"); t.close()
        with self.assertRaises(zipfile.BadZipFile):
            unpack(t.name, APP, tempfile.mkdtemp())

    def test_headers_that_disagree(self):
        f = make([("A.app/good", "1", R)])
        b = open(f, "rb").read().replace(b"A.app/good", b"A.app/evil", 1)
        open(f, "wb").write(b)
        with self.assertRaises(zipfile.BadZipFile):
            unpack(f, APP, tempfile.mkdtemp())


unittest.main()
