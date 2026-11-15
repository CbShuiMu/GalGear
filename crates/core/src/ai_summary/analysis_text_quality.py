"""首次自写读取返回正文和编码对照；诊断不另发模型请求，也不阻断已有产物。"""
import hashlib
import json
from pathlib import Path


def review_reader(agent, source, records):
    digest=hashlib.sha256(source.encode()).hexdigest()
    if digest in getattr(agent,'text_diagnostics_seen',set()): return None
    samples=[]
    def visit(value):
        if len(samples)>=6: return
        if isinstance(value,str) and len(value)>=5 and sum(ord(c)>127 for c in value)>=4:
            if value[:200] not in samples: samples.append(value[:200])
        elif isinstance(value,dict):
            for child in value.values(): visit(child)
        elif isinstance(value,list):
            for child in value: visit(child)
    for record in records[:16]:
        if record.get('output'): visit(json.loads(Path(record['output']).read_text('utf-8')))
        if len(samples)>=6: break
    # 纯命令样本不消耗同一读取器后续第一次真实正文的诊断机会。
    if not samples: return None
    alternatives=[]
    for sample in samples[:3]:
        variants=[]
        for original in ('cp932','gbk','utf-8'):
            try: raw=sample.encode(original)
            except UnicodeError: continue
            for target in ('cp932','gbk','utf-8'):
                if target==original: continue
                try: text=raw.decode(target)
                except UnicodeError: continue
                variants.append(dict(assumed_wrong_encoding=original,candidate_encoding=target,text=text))
        alternatives.append(dict(actual=sample,counterfactuals=variants))
    result=dict(decoder_sha256=digest,samples=samples,encoding_diagnostics=alternatives,
        advice='Compare actual dialogue with the referenced format encoding. Successful legacy decoding alone does not prove readability. These alternatives are diagnostic, not automatic corrections.')
    agent.emit('reader-quality-'+digest[:16]+'.json',result)
    if not hasattr(agent,'text_diagnostics_seen'): agent.text_diagnostics_seen=set()
    agent.text_diagnostics_seen.add(digest)
    return result
