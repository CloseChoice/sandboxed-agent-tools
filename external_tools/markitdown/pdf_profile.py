"""PDF-only orchestration around the unchanged upstream converter and CLI."""
import io
import os
from pathlib import Path
import re
from urllib.parse import unquote, urlparse

from ._base_converter import DocumentConverterResult
from ._stream_info import StreamInfo
from .converters._pdf_converter import PdfConverter
from ._exceptions import UnsupportedFormatException


class MarkItDown:
    def __init__(self, *, enable_plugins=False, **kwargs):
        if enable_plugins or kwargs:
            raise NotImplementedError('This WASI profile supports local PDF conversion; plugins and cloud backends are unavailable')
        self.converter = PdfConverter()

    def convert(self, source, **kwargs):
        if hasattr(source, 'read'):
            return self.convert_stream(source, **kwargs)
        name = os.fspath(source)
        if name.startswith('file://'):
            parsed = urlparse(name)
            if parsed.netloc not in ('', 'localhost'):
                raise ValueError('Only local file URLs are supported')
            name = unquote(parsed.path)
        elif '://' in name:
            raise NotImplementedError('Remote URLs are not supported by this WASI PDF profile')
        with open(name, 'rb') as stream:
            return self.convert_stream(stream, **kwargs)

    def convert_stream(self, stream, *, stream_info=None, keep_data_uris=False, **kwargs):
        if kwargs:
            raise NotImplementedError(f'Unsupported conversion options: {sorted(kwargs)}')
        data = stream.read(16 * 1024 * 1024 + 1)
        if len(data) > 16 * 1024 * 1024:
            raise ValueError('PDF input exceeds the 16 MiB file limit')
        if b'%PDF-' not in data[:1024]:
            raise UnsupportedFormatException('This WASI profile only supports PDF input')
        result = self.converter.convert(io.BytesIO(data), StreamInfo(extension='.pdf', mimetype='application/pdf'),
                                        keep_data_uris=keep_data_uris)
        # Same normalization as upstream MarkItDown._convert; no new formatting.
        result.text_content = '\n'.join(line.rstrip() for line in re.split(r'\r?\n', result.text_content))
        result.text_content = re.sub(r'\n{3,}', '\n\n', result.text_content)
        return result
