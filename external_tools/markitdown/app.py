"""Invoke the upstream CLI inside WASI; host policy remains the security boundary."""
import codecs
import encodings.cp1252
import encodings.latin_1
import encodings.utf_16_be
import encodings.utf_8
import importlib.metadata
import locale
import gzip
import pickle
import os
import sys
import traceback
import unicodedata

import markitdown.__main__
import pdfminer.high_level
import pdfplumber
import wit_world


class WitWorld(wit_world.WitWorld):
    def run(self, args: list[str]) -> int:
        try:
            if 'PWD' in os.environ:
                os.chdir(os.environ['PWD'])
            sys.argv = ['markitdown', *args]
            markitdown.__main__.main()
            return 0
        except SystemExit as error:
            if error.code is None:
                return 0
            if isinstance(error.code, int):
                return error.code
            print(error.code, file=sys.stderr)
            return 1
        except BaseException:
            traceback.print_exc()
            return 1
