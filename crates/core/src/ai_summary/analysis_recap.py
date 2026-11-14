"""从实际读取结果裁出存档之前的窗口，再在独立上下文中生成提要。

选择器不认定执行语义：字幕精确匹配仍是候选，坐标转换由模型明确说明。
裁剪和来源绑定由宿主执行，路线探索中读到的后续剧情不会进入提要请求。
"""
import json


def same_caption(value, target):
    """仅忽略显示层首尾空白；保留标点、正文内空白及原始字段供审计。"""
    return isinstance(value,str) and isinstance(target,str) and value.strip() == target.strip()


def contains(value, target):
    if isinstance(value, str):
        return same_caption(value, target)
    if isinstance(value, (list, dict)):
        return any(contains(v, target) for v in (value.values() if isinstance(value,dict) else value))
    return False


def through_match(value, target):
    """在匹配到的原始字符串后停止，连同同一块中之后的消息一并移除。"""
    if same_caption(value, target):
        return value, True
    if isinstance(value, dict):
        result={}
        for key, child in value.items():
            part, found=through_match(child,target)
            result[key]=part
            if found:return result,True
        return result,False
    if isinstance(value,list):
        result=[]
        for child in value:
            part,found=through_match(child,target)
            result.append(part)
            if found:return result,True
        return result,False
    return value,False


def select_context(agent, args):
    known={}
    for index in agent.batches:
        batch=json.loads(agent.host.path('work',index).read_text('utf-8'))
        for record in batch['records']:
            if 'output' in record:known[record['output']]=dict(record,root=batch['input_root'])
    def resolve(name, is_save):
        if name in known:
            return name
        matches=[path for path, record in known.items()
                 if record['root'].startswith('save')==is_save
                 and name in (record['input'], record.get('name'), record['input'].replace('\\','/').rsplit('/',1)[-1])]
        # 重复读取同一原文件时只选最新投影；不同归档的同名脚本仍须消歧。
        identities={known[path]['input'] for path in matches}
        if len(identities)==1:
            return matches[-1]
        available=[dict(source=r.get('name',r['input']),json=path) for path,r in known.items()
                   if r['root'].startswith('save')==is_save]
        raise ValueError(('Read the original save/script with read_resources first; byte inspection alone is not a registered reader. ' if not available else '')+'Select a unique original source name or reader JSON path. Candidates: '+json.dumps(available[:12],ensure_ascii=False))
    save=resolve(args.get('save_file') or agent.preferred_save,True)
    script=resolve(args['script_file'],False)
    if not known[save]['root'].startswith('save') or known[script]['root'] not in ('game','work'):
        raise ValueError('Anchor must come from a save root and context from a game/script read')
    if agent.preferred_save and known[save]['input']!=agent.preferred_save:
        raise ValueError('The latest progress save was decoded successfully: '+agent.preferred_save+'. Use its actual fields/last caption; a historical bookmark/global index cannot replace this current save.')
    def value(path,pointer):
        document=json.loads(agent.host.path('work',path).read_text('utf-8'))
        return agent.api['json_pointer_value'](document,pointer)
    anchor=value(save,args['save_pointer'])
    data=value(script,args['script_pointer'])
    if not isinstance(data,(dict,list)):
        raise ValueError('script_pointer must select an ordered record array/object, not a scalar')
    pairs=list(data.items()) if isinstance(data,dict) else list(enumerate(data))
    offset=args.get('offset',0)
    if type(offset) is not int or offset not in (-1,0,1):
        raise ValueError('Coordinate offset must be -1, 0, or 1, with a format explanation')
    if isinstance(anchor,str) and len(anchor.strip())>=4:
        hits=[i for i,(_,v) in enumerate(pairs) if contains(v,anchor)]
    elif type(anchor) is int:
        coordinate=anchor+offset
        field=args.get('coordinate_field')
        hits=([i for i,(_,v) in enumerate(pairs) if isinstance(v,dict) and v.get(field)==coordinate]
              if field else [coordinate] if 0<=coordinate<len(pairs) else [])
    else:
        raise ValueError('Save pointer must select a real caption (>=4 characters) or integer coordinate')
    if len(hits)!=1:
        raise ValueError('Anchor must match exactly one record in this script collection; matches='+str(len(hits))+'. Narrow script_pointer or choose the actual coordinate field.')
    end=hits[0]
    selected=pairs[max(0,end-23):end+1]
    if isinstance(anchor,str):
        last,_=through_match(selected[-1][1],anchor)
        selected[-1]=(selected[-1][0],last)
    context=dict(version=1,save_input=known[save]['input'],save_sha256=known[save]['sha256'],
        custom_decoder=any(known[path].get('reader')=='custom' for path in (save,script)),
        script_input=known[script]['input'],script_sha256=known[script]['sha256'],
        anchor=anchor,selection=args,record_index=end,records=[dict(key=k,value=v) for k,v in selected],
        caption_normalization='leading/trailing whitespace only; original strings preserved',
        boundary='Only the selected script-order window up to the save anchor is included. Candidate context, not verified reading history.')
    if len(json.dumps(context,ensure_ascii=False).encode())>60000:
        raise ValueError('Context exceeds 60 KiB. Select a message collection, not whole scene objects.')
    path='recap-context.json'
    agent.emit(path,context)
    agent.recap_context=context
    return dict(context=path,anchor=anchor,record_index=end,records=len(selected),
                preview=context['records'][-2:],next_action='Continue route analysis; finish_analysis will generate the recap in a fresh context using ONLY this window.')


def summarize(agent):
    context=agent.recap_context
    if not context:
        raise ValueError('Call select_recap_context first. A whole-game synopsis cannot replace a current recap.')
    system='''Write a substantive game recap and current progress in the requested language using ONLY the supplied script window and save anchor. This is a fresh restricted context; do not use prior knowledge of this game. The window ends at the supplied anchor: absolutely no events, intentions or actions after that point. Summarize coherent preceding events and the situation at the boundary. Script order is not proven reading history; briefly label reconstruction as inferred. Do not invent speakers for unnamed narration. Omit meaningless repeated fillers but retain meaningful responses. Write natural prose without parser metadata, offsets or filenames in the recap. Describe story events, not rendering instructions, dialogue boxes, sprite changes, title labels or screen transitions. Do not infer a location change merely from a background image change. current_progress may briefly explain the location and candidate boundary. Do not write a global synopsis, future choices/endings, or a disclaimer-only response. Avoid graphic detail. Return deliver_summary with recap and current_progress; use real paragraph newlines.'''
    tool=agent.api['tool']('deliver_summary','Return only the bounded current recap and progress.',
                          dict(recap=dict(type='string'),current_progress=dict(type='string')),['recap','current_progress'])
    messages=[dict(role='system',content=system),dict(role='user',content=json.dumps(dict(
        language=agent.config['language'],context=context),ensure_ascii=False))]
    for attempt in range(2):
        response=agent.complete(messages,[tool],phase='recap',max_tokens=4000)
        message=response['message']
        calls=message.get('tool_calls') or []
        try:
            result=json.loads(calls[0]['function']['arguments']) if calls else json.loads(message['content'])
            if (not isinstance(result,dict) or any(not isinstance(result.get(k),str) or len(result[k].strip())<30
                                                  for k in ('recap','current_progress'))):
                raise ValueError('Both fields must be substantive prose')
            return {k:result[k] for k in ('recap','current_progress')}
        except (ValueError,KeyError,IndexError,TypeError):
            messages.append(dict(role='user',content='Return valid deliver_summary arguments with substantive recap/current_progress, using only this same bounded evidence.'))
    raise ValueError('Recap response invalid after one format repair')
