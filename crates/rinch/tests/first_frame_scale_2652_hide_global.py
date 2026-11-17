"""A Wayland proxy that hides globals from its clients (issue #2652's live test).

    python3 -I first_frame_scale_2652_hide_global.py UPSTREAM LISTEN INTERFACE...

UPSTREAM and LISTEN are socket paths. Each client of LISTEN gets its own
connection to UPSTREAM; everything passes through unchanged, file descriptors
included, except `wl_registry.global` events that announce one of the
INTERFACEs, which are dropped, so the client never binds them. The test uses it
to run a client on a compositor "without" wp_fractional_scale_manager_v1, which
no compositor at hand lets you turn off.

Runs until killed. Prints "ready" on stdout once LISTEN accepts connections.
"""

import array
import os
import socket
import struct
import sys
import threading

MAX_FDS = 28


def recv(sock):
    fds = array.array("i")
    data, ancdata, _flags, _addr = sock.recvmsg(
        65536, socket.CMSG_LEN(MAX_FDS * fds.itemsize)
    )
    for level, kind, payload in ancdata:
        if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
            fds.frombytes(payload[: len(payload) - (len(payload) % fds.itemsize)])
    return data, list(fds)


def send(sock, data, fds):
    while data or fds:
        anc = (
            [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds[:MAX_FDS]))]
            if fds
            else []
        )
        sent = sock.sendmsg([data], anc)
        for fd in fds[:MAX_FDS]:
            os.close(fd)
        fds = fds[MAX_FDS:]
        data = data[sent:]


def messages(buffer):
    """Split whole messages off `buffer`: (list of (object, opcode, bytes), rest)."""
    out = []
    while len(buffer) >= 8:
        obj, word = struct.unpack_from("=II", buffer)
        size = word >> 16
        if size < 8 or len(buffer) < size:
            break
        out.append((obj, word & 0xFFFF, buffer[:size]))
        buffer = buffer[size:]
    return out, buffer


def interface_of_global(message):
    # wl_registry.global(name: uint, interface: string, version: uint)
    length = struct.unpack_from("=I", message, 12)[0]
    return message[16 : 16 + length - 1].decode()


def serve(client, upstream_path, hidden):
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.connect(upstream_path)
    registries = set()
    lock = threading.Lock()

    def client_to_server():
        pending = b""
        try:
            while True:
                data, fds = recv(client)
                if not data and not fds:
                    break
                pending += data
                parsed, pending = messages(pending)
                for obj, opcode, body in parsed:
                    # wl_display (object 1).get_registry (opcode 1, new_id)
                    if obj == 1 and opcode == 1:
                        with lock:
                            registries.add(struct.unpack_from("=I", body, 8)[0])
                send(server, data, fds)
        except OSError:
            pass
        finally:
            for s in (server, client):
                try:
                    s.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def server_to_client():
        pending = b""
        try:
            while True:
                data, fds = recv(server)
                if not data and not fds:
                    break
                pending += data
                parsed, pending = messages(pending)
                kept = b""
                for obj, opcode, body in parsed:
                    with lock:
                        is_global = obj in registries and opcode == 0
                    if is_global and interface_of_global(body) in hidden:
                        continue
                    kept += body
                send(client, kept, fds)
        except OSError:
            pass
        finally:
            for s in (server, client):
                try:
                    s.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    threading.Thread(target=client_to_server, daemon=True).start()
    threading.Thread(target=server_to_client, daemon=True).start()


def main():
    upstream, listen_path, hidden = sys.argv[1], sys.argv[2], set(sys.argv[3:])
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    if os.path.exists(listen_path):
        os.unlink(listen_path)
    listener.bind(listen_path)
    listener.listen()
    print("ready", flush=True)
    while True:
        client, _ = listener.accept()
        serve(client, upstream, hidden)


if __name__ == "__main__":
    main()
