"""为文本脚本提供跨文件的控制语句线索；不推断执行顺序或解锁语义。"""
import re


ENTRY = re.compile(r'true.?end|grand|after.?story|unlock|opened|解放|解禁|真エンド', re.I)
STATE = re.compile(r'clear|game.?start|\.visible|\.enabled|クリア', re.I)


def text_hints(text):
    if not isinstance(text, str):
        return []
    hits=[]
    lines=text.splitlines()
    for line, value in enumerate(lines, 1):
        value=value.strip()
        if value.startswith(('//',';','#','--')):
            continue
        priority=0 if ENTRY.search(value) else 1 if STATE.search(value) else None
        if priority is not None:
            if re.search(r'\b(?:function|def)\b',value) and line<len(lines):
                value+='\n'+lines[line].strip()
            hits.append(dict(line=line, text=value[:400], priority=priority))
    return sorted(hits,key=lambda item:(item['priority'],item['line']))[:12]


def batch_hints(records, limit=8000):
    """Read all per-file projections, then bound only the preview, not evidence."""
    sources=[]
    used=0
    for record in sorted(records,key=lambda r:min((h['priority'] for h in r.get('control_hints',[])),default=2)):
        hints=[]
        for hint in record.get('control_hints',[]):
            size=len(hint['text'].encode('utf-8'))+len(record.get('name','').encode('utf-8'))+100
            if used+size>limit:
                break
            hints.append(dict(line=hint['line'],text=hint['text']))
            used+=size
        if hints:
            sources.append(dict(name=record['name'],output=record['output'],pointer='/text',hints=hints))
    return sources
