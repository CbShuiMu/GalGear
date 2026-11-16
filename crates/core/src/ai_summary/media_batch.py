"""只接受已通过试提取的计划，在无模型、无凭据的独立进程中批量执行。"""
import json
from pathlib import Path
import runpy
import time

MEDIA = runpy.run_path(str(Path(__file__).with_name('media_agent.py')))
PLAN = MEDIA['PLAN']


def run(config,api):
    if any(config.get(key) for key in ('token','provider','model')):
        raise ValueError('Batch workers must not receive model credentials or configuration')
    work=Path(config['work']).resolve()
    plan=json.loads((work/'media-plan.json').read_text('utf-8'))
    if plan.get('schema')!=1 or plan.get('kind')!=config.get('kind') or not plan.get('recipes'):
        raise ValueError('Invalid tested media plan')
    agent=MEDIA['MediaAgent'](config,api,batch=True)
    agent.host.documents=plan['documents']
    agent.input_stamps={path:tuple(stamp) for path,stamp in plan['input_stamps'].items()}
    agent.check_inputs()
    agent.warnings=list(plan['warnings'])
    agent.batch_total=len(plan['recipes'])
    agent.batch_completed=0
    agent.progress('extracting')
    agent.state()
    try:
        for recipe in plan['recipes']:
            # 重新列举源成员，使规划后修改的归档/松散资源不能沿用旧样本结论。
            current=PLAN['listing'](agent,recipe)
            if current!=recipe['members']:
                raise ValueError('Media selection changed after sample validation')
            if recipe['mode']=='archive':
                result=agent.host._dispatch('run_skill',dict(command='x',archive=recipe['path'],filter=recipe['pattern'],filter_syntax='glob'))
                if result.get('error') or result.get('exit_code',0):
                    raise ValueError(result.get('error') or 'Batch archive extraction failed')
                inventory=json.loads(agent.host.path('work',result['member_inventory']).read_text('utf-8'))
                if set(name.replace('\\','/') for name in inventory['members'])!=set(name.replace('\\','/') for name in current):
                    raise ValueError('Batch archive extraction omitted selected members')
                root='work'
                directory=agent.host.path('work',result['output'])
            else:
                root='game'
                directory=agent.host.path('game',recipe['path'])
            samples={item['name'].replace('\\','/'):item['sha256'] for item in recipe['samples']}
            for name in samples:
                path=agent.files_api['checked'](directory/name,directory)
                if agent.files_api['digest'](path)!=samples[name]:
                    raise ValueError('Validated sample bytes changed before batch extraction')
            for offset in range(0,len(current),32):
                if time.monotonic()>=agent.deadline:
                    raise TimeoutError('Batch media extraction exceeded its runtime limit')
                names=[name.replace('\\','/') for name in current[offset:offset+32]]
                relative=directory.relative_to(agent.host.roots[root]).as_posix()
                if agent.kind=='cg':
                    agent.files_api['convert_images'](agent,dict(root=root,path=relative,_paths=names,mode='files'))
                else:
                    for name in names:
                        PLAN['audio_probe'](agent.files_api['checked'](directory/name,directory))
                    agent.files_api['export'](agent,dict(root=root,path=relative,_paths=names))
                agent.progress('extracting')
            agent.batch_completed+=1
            agent.progress('extracting')
        result=agent.files_api['finish'](agent,dict(scope=plan['scope'],warnings=[]))
        agent.progress('extracting','completed')
        agent.state('completed')
        return result
    except Exception as error:
        agent.state('failed',error=str(error)[:1200])
        raise
