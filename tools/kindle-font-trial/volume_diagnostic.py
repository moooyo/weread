"""Bind one scratch-only diagnostic helper and validate its public capability result."""

import json
from pathlib import Path


POLICY = 'selector-diagnostic-v1'
FILESYSTEM = 'vfat exfat fuse ext xfs tmpfs other unknown'.split()
ADMISSION = ('verified fdinfo_open fdinfo_read mnt_id_missing mnt_id_format mnt_id_mismatch owner_opath '
             'owner_proc_reopen owner_identity scratch_owner scratch_mkdir scratch_subdir').split()
COMMON = ('not_run supported create_left create_right marker_write marker_fsync marker_stat scratch_collision '
          'out_fsync_left out_fsync_right back_fsync_left back_fsync_right post_stat post_inode post_nlink '
          'post_owner_mode post_size post_mtime post_open post_content').split()
EXCHANGE = COMMON + ['exchange_out', 'exchange_back']
HARDLINK = COMMON + ['link_create', 'link_fsync', 'rename_out', 'rename_back']


def program(size):
    if type(size) is not int or not 52 <= size <= 128 * 1024:
        raise ValueError('invalid_diagnostic_size')
    source = Path(__file__).with_name('volume-diagnostic.sh').read_text(encoding='ascii')
    if source.count('@HELPER_BYTES@') != 1:
        raise ValueError('invalid_diagnostic_program')
    return source.replace('@HELPER_BYTES@', str(size))


def observation(raw):
    if not isinstance(raw, bytes) or not raw.endswith(b'\n') or len(raw) > 640 or raw.count(b'\n') != 1:
        raise ValueError('invalid_diagnostic_receipt')

    def unique(items):
        if len(dict(items)) != len(items):
            raise ValueError('duplicate_diagnostic_field')
        return dict(items)

    data = json.loads(raw.decode('ascii'), object_pairs_hook=unique)
    kinds = {'admission': ADMISSION, 'exchange': EXCHANGE, 'hardlink': HARDLINK}
    if (not isinstance(data, dict) or set(data) != {'policy', 'filesystem', 'clean', *kinds, *(key+'_errno' for key in kinds)}
            or data['policy'] != POLICY or data['filesystem'] not in FILESYSTEM or type(data['clean']) is not bool):
        raise ValueError('invalid_diagnostic_fields')
    for key, names in kinds.items():
        code = data[key+'_errno']
        if (data[key] not in names or type(code) is not int or not 0 <= code <= 4095
                or (data[key] in ('not_run','supported','verified') and code != 0)):
            raise ValueError('invalid_diagnostic_stage')
    return data
