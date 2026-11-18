"""CPython runs inside Wasm. Output capture is presentation, not the sandbox."""
import contextlib
import io
import traceback

# componentize-py snapshots imports at build time. The portable stdlib profile
# is exercised through WASI in test.py and test_xml.py; importing alone is insufficient
# for modules with lazy dependencies.
import argparse
import collections
import csv
import datetime
import decimal
import fractions
import functools
import hashlib
import itertools
import json
import math
import os
import pathlib
import random
import re
import shutil
import socket
import sqlite3
import statistics
import sys
import tarfile
import tomllib
import zipfile

# XML parsers and lazy DOM/SAX drivers must be reachable in the snapshot too.
import xml.etree.ElementTree
import xml.etree.ElementInclude
import xml.etree.cElementTree
import xml.dom.minidom
import xml.dom.expatbuilder
import xml.dom.pulldom
import xml.dom.xmlbuilder
import xml.sax
import xml.sax.expatreader
import xml.parsers.expat
# urllib loads this lazily for XML file-URI references.
import mimetypes

# Lazy dependencies used by the profile's supported operations.
import _strptime
import encodings.cp437
import encodings.ascii
import encodings.latin_1
import encodings.utf_16
import encodings.utf_16_be
import encodings.utf_16_le
import gzip
import locale

import encodings.idna
import sat_http
sat_http.install()

import wit_world
from componentize_py_types import Err


class WitWorld(wit_world.WitWorld):
    def run(self, code: str | None, script: str | None) -> str:
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            try:
                if (code is None) == (script is None):
                    raise ValueError("provide exactly one of -c or a script path")
                # Presentation/path setup only; the host enforces file grants.
                if "PWD" in os.environ:
                    os.chdir(os.environ["PWD"])
                namespace = {"__name__": "__main__"}
                if script is not None:
                    filename = script
                    namespace["__file__"] = script
                    sys.argv = [script]
                    # Compile bytes so Python source encoding declarations work.
                    with open(script, "rb") as source:
                        code = source.read()
                else:
                    filename = "<string>"
                    sys.argv = ["-c"]
                exec(compile(code, filename, "exec"), namespace)
            except SystemExit as error:
                if error.code not in (None, 0):
                    raise Err(output.getvalue() + f"SystemExit: {error.code}")
            except BaseException:
                raise Err(output.getvalue() + traceback.format_exc())
        return output.getvalue()
