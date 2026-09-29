"""Completion evidence required before accepting a durable benchmark process."""
import pathlib
import re


def validate_durable_output(path, backend, appends, fragment):
    text = pathlib.Path(path).read_text()
    if ',write_failed,' in text:
        raise ValueError('durable workload reported write_failed')
    if not re.search(r'^test result: ok\.', text, re.MULTILINE):
        raise ValueError('durable workload did not report test success')
    identity = [backend, str(appends), str(fragment), '1']
    rows = [line.split(',') for line in text.splitlines()]
    matching = [row for row in rows if row[:4] == identity]
    equivalence = [row for row in matching if row[4:5] == ['equivalence']]
    restart = [row for row in matching if row[4:5] == ['restart']]
    if len(equivalence) != 1 or len(restart) != 1:
        raise ValueError('durable workload missing unique equivalence/restart rows for expected case')
    equivalence, restart = equivalence[0], restart[0]
    if (len(equivalence) != 13
            or equivalence[5::2] != ['versions', 'data_bytes', 'versions_digest', 'replay_digest']
            or equivalence[6] != str(appends + 1)
            or not equivalence[8].isdigit()
            or not re.fullmatch(r'[0-9a-f]{16}', equivalence[10])
            or equivalence[10] != equivalence[12]):
        raise ValueError('durable workload equivalence count or digests are invalid')
    if (len(restart) != 9 or restart[5::2] != ['versions_digest', 'replay_digest']
            or restart[6] != equivalence[10] or restart[8] != equivalence[12]):
        raise ValueError('durable workload restart digests do not match equivalence')
