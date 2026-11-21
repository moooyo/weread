"""Validate a public mount-table classification without retaining raw kernel rows."""

import json


COUNT = {'zero', 'one', 'multiple'}
FIELDS = {
    'policy': {'mount-table-read-v1'}, 'table': {'ok', 'open_error', 'read_error'},
    'fd': set('ok dir_open fdinfo_read missing format not_observed'.split()),
    'base_rows': COUNT, 'base_fd_rows': COUNT, 'fd_rows': COUNT,
    'base_shape': set('none root subdir escaped nonabsolute oversize ambiguous'.split()),
    'first_error': set('none table_open table_read row_limit line_limit missing_field mount_id parent_id device_field root_nonabsolute point_nonabsolute base_duplicate base_escaped base_root_limit base_separator base_absent target_root_limit target_separator base_changed'.split()),
    'error_scope': set('none volume other indeterminate'.split()),
    'repeat': {'same', 'different', 'unavailable'}, 'seek': {'not_observed'},
    'propagation': {'private', 'shared_or_slave', 'unknown'},
}


def observation(raw):
    if not isinstance(raw,bytes) or not raw.isascii() or not raw.endswith(b'\n') or raw.count(b'\n') != 1 or len(raw) > 1024:
        raise ValueError('invalid_mount_table_packet')
    def unique(items):
        result = dict(items)
        if len(result) != len(items): raise ValueError('duplicate_mount_table_key')
        return result
    try:
        result = json.loads(raw,object_pairs_hook=unique)
    except RecursionError:
        raise ValueError('invalid_mount_table_packet') from None
    if (not isinstance(result,dict) or result.keys() != FIELDS.keys() or
            any(type(value) is not str or value not in FIELDS[key] for key,value in result.items())):
        raise ValueError('invalid_mount_table_classification')
    return result
