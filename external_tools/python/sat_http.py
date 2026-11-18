"""urllib HTTP/HTTPS transport using host-enforced WASI HTTP permissions."""
import email.message
import http.client
import io
import math
import socket
import urllib.error
import urllib.parse
import urllib.request
import urllib.response

from componentize_py_types import Err, Ok
from wit_world.imports import outgoing_handler, streams, types

MAX_BODY_BYTES = 8 * 1024 * 1024


def _http_error(error):
    if isinstance(error, types.ErrorCode_HttpRequestDenied):
        return urllib.error.URLError("HTTP request denied by SAT host policy")
    return urllib.error.URLError(type(error).__name__)


def _response(future, url):
    with future:
        with future.subscribe() as ready:
            ready.block()
        result = future.get()
        if not isinstance(result, Ok) or not isinstance(result.value, Ok):
            error = result.value.value if isinstance(result, Ok) else None
            raise _http_error(error)
        with result.value.value as response:
            status = response.status()
            headers = email.message.Message()
            with response.headers() as fields:
                for name, value in fields.entries():
                    headers[name] = value.decode("latin-1")
            data = bytearray()
            with response.consume() as body:
                with body.stream() as source:
                    while True:
                        try:
                            chunk = source.blocking_read(min(65536, MAX_BODY_BYTES - len(data) + 1))
                        except Err as error:
                            if isinstance(error.value, streams.StreamError_Closed):
                                break
                            raise urllib.error.URLError("HTTP response body stream failed") from error
                        if not chunk:
                            break
                        data.extend(chunk)
                        if len(data) > MAX_BODY_BYTES:
                            raise urllib.error.URLError("HTTP response exceeds the 8 MiB buffer limit")
    result = urllib.response.addinfourl(io.BytesIO(data), headers, url, status)
    result.msg = http.client.responses.get(status, "")
    return result


class WasiHTTPHandler(urllib.request.HTTPHandler):
    def http_open(self, req):
        parsed = urllib.parse.urlsplit(req.full_url)
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            raise urllib.error.URLError("expected an HTTP or HTTPS URL")
        if parsed.username is not None or parsed.password is not None:
            raise urllib.error.URLError("credentials in URLs are unsupported")
        data = req.data
        if data is not None and not isinstance(data, (bytes, bytearray, memoryview)):
            raise urllib.error.URLError("HTTP request data must be bytes")
        if data is not None and len(data) > MAX_BODY_BYTES:
            raise urllib.error.URLError("HTTP request exceeds the 8 MiB buffer limit")
        # Authority supplies Host; the runtime validates it and owns transport headers.
        excluded = {"host", "connection", "transfer-encoding", "keep-alive", "upgrade"}
        headers = [(name.lower(), str(value).encode("latin-1"))
                   for name, value in req.header_items() if name.lower() not in excluded]
        try:
            request = types.OutgoingRequest(types.Fields.from_list(headers))
            method = req.get_method()
            method_type = getattr(types, "Method_" + method.title(), None)
            request.set_method(method_type() if method_type else types.Method_Other(method))
            request.set_scheme(types.Scheme_Https() if parsed.scheme == "https" else types.Scheme_Http())
            request.set_authority(parsed.netloc)
            request.set_path_with_query((parsed.path or "/") + ("?" + parsed.query if parsed.query else ""))
            options = None
            timeout = req.timeout
            if timeout is not socket._GLOBAL_DEFAULT_TIMEOUT and timeout is not None:
                if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
                    raise ValueError("timeout must be a positive finite number")
                duration = int(timeout * 1_000_000_000)
                options = types.RequestOptions()
                options.set_connect_timeout(duration)
                options.set_first_byte_timeout(duration)
                options.set_between_bytes_timeout(duration)
            body = request.body()
            if data is None:
                types.OutgoingBody.finish(body, None)
                return _response(outgoing_handler.handle(request, options), req.full_url)
            # Dispatch before writing so the host consumes data as the guest produces it.
            future = outgoing_handler.handle(request, options)
            try:
                with body.write() as output:
                    for offset in range(0, len(data), 4096):
                        output.blocking_write_and_flush(bytes(data[offset:offset + 4096]))
                types.OutgoingBody.finish(body, None)
            except BaseException:
                future.__exit__(None, None, None)
                raise
            return _response(future, req.full_url)
        except Err as error:
            raise _http_error(error.value) from error

    https_open = http_open


def install():
    # Redirects go through urllib again; every hop is checked by the host.
    # Raw sockets and TLS contexts are not used by this adapter.
    urllib.request.install_opener(urllib.request.build_opener(
        urllib.request.ProxyHandler({}), WasiHTTPHandler()))
