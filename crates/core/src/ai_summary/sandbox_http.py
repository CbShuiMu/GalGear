"""仅桥接 HTTP 请求；文件权限始终由宿主创建的 Windows LPAC 控制。"""
import base64
import email.message
import http
import io
import json
import os
import threading
import urllib.error
import urllib.request
import urllib.response

MAX_FRAME = 96 * 1024 * 1024


def install():
    name = os.environ.get('GALGEAR_AI_HTTP_HANDLE')
    if os.name != 'nt' or not name:
        return

    import msvcrt
    descriptor = None
    lock = threading.Lock()

    class BrokerHandler(urllib.request.BaseHandler):
        handler_order = 100

        def http_open(self, request):
            timeout = request.timeout if isinstance(request.timeout, (int, float)) else 120
            payload = json.dumps(dict(url=request.full_url, method=request.get_method(),
                headers=request.header_items(), body=base64.b64encode(request.data or b'').decode('ascii'),
                use_proxy=getattr(request, 'galgear_use_proxy', True),
                timeout=max(1, min(180, int(timeout)))), ensure_ascii=True).encode('ascii') + b'\n'
            if len(payload) > MAX_FRAME:
                raise urllib.error.URLError('AI HTTP request exceeded limit')
            nonlocal descriptor
            with lock:
                if descriptor is None:
                    descriptor = msvcrt.open_osfhandle(int(name), os.O_BINARY | os.O_RDWR)
                with os.fdopen(os.dup(descriptor), 'r+b', buffering=0) as pipe:
                    remaining = memoryview(payload)
                    while remaining:
                        written = pipe.write(remaining)
                        if not written:
                            raise urllib.error.URLError('AI HTTP channel closed')
                        remaining = remaining[written:]
                    reader = io.BufferedReader(pipe)
                    encoded = reader.readline(MAX_FRAME + 1)
                    if len(encoded) > MAX_FRAME or not encoded.endswith(b'\n'):
                        raise urllib.error.URLError('Invalid AI HTTP response')
                    pipe.write(b'ok\n')
                    reply = json.loads(encoded)
            if reply.get('error'):
                raise urllib.error.URLError(reply['error'])
            headers = email.message.Message()
            for key, value in reply['headers']:
                headers[key] = value
            response = urllib.response.addinfourl(io.BytesIO(base64.b64decode(reply['body'], validate=True)),
                                                 headers, request.full_url, reply['status'])
            response.msg = http.HTTPStatus(reply['status']).phrase if reply['status'] in http.HTTPStatus._value2member_map_ else ''
            return response

        https_open = http_open

    original = urllib.request.build_opener

    def build_opener(*handlers):
        # 模型传输和源码下载各自安装 NoRedirect，保留这些策略并让它们共用网络通道。
        return original(urllib.request.ProxyHandler({}), BrokerHandler(), *handlers)

    urllib.request.build_opener = build_opener
    urllib.request.install_opener(build_opener())
