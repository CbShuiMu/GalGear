"""本轮读取结果的来源解析：统一 Windows 路径别名，保留跨归档歧义。"""
import json
from pathlib import Path


def normalized_path(value):
    return str(value).replace('\\','/').removeprefix('./').casefold()


def reader_records(agent):
    known={}
    for index in agent.batches:
        batch=json.loads(agent.host.path('work',index).read_text('utf-8'))
        for record in batch.get('records',[]):
            if record.get('output'):
                known[record['output']]=dict(record,root=batch.get('input_root','game'))
    return known


def source_aliases(agent,record,output):
    source=record['input'];root=record['root'];name=record.get('name','')
    values=[source,name,root+'/'+name,source.replace('\\','/').rsplit('/',1)[-1],output,str(agent.work/output)]
    return {normalized_path(value) for value in values if value}


def resolve_reader(agent,known,name,roots):
    """明确输出路径优先；同一原文件的重复读取取最新，不合并不同归档同名成员。"""
    target=normalized_path(name)
    allowed={path:record for path,record in known.items() if record['root'] in roots}
    def output_aliases(path):
        absolute=agent.work/path
        aliases={normalized_path(path),normalized_path(absolute)}
        if absolute.is_relative_to(agent.work):aliases.add(normalized_path(absolute.relative_to(agent.work)))
        return aliases
    explicit=[path for path in allowed if target in output_aliases(path)]
    if len(explicit)==1:return explicit[0]
    matches=[path for path,record in allowed.items() if target in source_aliases(agent,record,path)]
    if matches and len({normalized_path(allowed[path]['input']) for path in matches})==1:
        return matches[-1]
    available=[dict(source=r.get('name',r['input']),json=path) for path,r in allowed.items()]
    raise ValueError('Select a unique original source name or reader JSON path from registered reads. Candidates: '+json.dumps(available[:12],ensure_ascii=False))
