"""Parse bounded, non-authoritative archive receipt evidence from a closed install."""

import re


HEADER_RECEIVED = b'WEREAD_STOCK_INSTALL_HEADER_COMPLETE'
PREFIX = b'WEREAD_STOCK_RECEIVE_PROGRESS'
MAX_ARCHIVE = 180 * 1024**2
RECORD = re.compile(rb'WEREAD_STOCK_RECEIVE_PROGRESS ([0-9a-f]{16}) '
                    + rb' '.join([rb'(0|[1-9][0-9]{0,18})'] * 5)
                    + rb' (receiving|eof|overrun|read_error|write_error)\n')


def checked_progress(stderr, label, expected_bytes):
    """Return only a visit-bound snapshot, never the original stderr."""
    if type(expected_bytes) is not int or not 0 < expected_bytes <= MAX_ARCHIVE:
        return None
    lines = stderr.splitlines(keepends=True)
    headers = [i for i, line in enumerate(lines) if line == HEADER_RECEIVED + b'\n']
    rows = [i for i, line in enumerate(lines) if line.startswith(PREFIX)]
    if len(headers) != 1 or len(rows) != 1 or not headers[0] < rows[0] < len(lines)-1:
        return None
    if not (row := RECORD.fullmatch(lines[rows[0]])) or row[1] != label.encode('ascii'):
        return None
    started, sampled, last, received, written = map(int, row.groups()[1:6])
    state = row[7].decode('ascii')
    if (not 0 < started <= sampled < 2**63 or not 0 <= written <= received <= expected_bytes + 1
            or (received == 0 and last != 0) or (received > 0 and not started <= last <= sampled)):
        return None
    if ((state in ('eof', 'read_error') and (written != received or received > expected_bytes))
            or (state == 'overrun' and (written != received or received != expected_bytes + 1))
            or (state == 'write_error' and written == received)):
        return None
    return {'clock': 'CLOCK_MONOTONIC', 'archive_bytes': expected_bytes,
            'started_ns': started, 'sampled_ns': sampled, 'last_data_read_ns': last or None,
            'bytes_read': received, 'bytes_written': written, 'state': state,
            'eof_observed': state == 'eof'}
