"""复核 #678 的完整批次、同场景摘要、工作量与有限结构实验。"""
import argparse,ast,collections,json,math,re,statistics
from pathlib import Path
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory',type=Path)
parser.add_argument('--models-dir',type=Path)
parser.add_argument('--work-log',type=Path)
args=parser.parse_args()
root=args.directory
rows=[];digests=collections.defaultdict(set)
def quantiles(a):
    a=sorted(a)
    return {f'p{p}':a[math.ceil(len(a)*p/100)-1] for p in [50,95,99]}|{'maximum':a[-1]}
for round in range(1,4):
    for line in (root/f'release-{round}.log').read_text(encoding='utf-8-sig').splitlines():
        if 'parking-command case=' not in line:continue
        m=re.search(r'Case \{ active: (\d+), parked: (\d+), commands: (\d+), success_percent: (\d+), order: (\d+) \} samples=(\d+) warmup=(\d+) digest=([0-9a-f]+) batch_ns=(\[.*?\]) commands_ns=(\[.*\])',line)
        assert m,line
        active,parked,commands,percent,order,samples,warmup=map(int,m.group(1,2,3,4,5,6,7))
        assert samples==16 and warmup==2
        key=f'{active}/{active+parked}/{commands}/{percent}/{order}'
        digests[key].add(m[8])
        batches=ast.literal_eval(m[9]); calls=ast.literal_eval(m[10].replace('true','True').replace('false','False'))
        assert len(batches)==samples and len(calls)==samples*commands
        assert sum(ok for ok,_ in calls)==samples*commands*percent//100
        row=dict(case=key,round=round,active=active,parked=parked,commands=commands,success_percent=percent,order=order,batch=quantiles(batches),call=quantiles([ns for _,ns in calls]))
        for ok,label in [(True,'success'),(False,'rejection')]:
            values=[ns for success,ns in calls if success==ok]
            row[label]=quantiles(values) if values else None
        rows.append(row)
assert len(rows)==102 and len(digests)==34 and all(len(x)==1 for x in digests.values())
summary=[]
for key in digests:
    group=[r for r in rows if r['case']==key]
    assert len(group)==3 and {r['round'] for r in group}=={1,2,3}
    result={k:group[0][k] for k in ['case','active','parked','commands','success_percent','order']}
    for phase in ['batch','call','success','rejection']:
        values=[r[phase] for r in group if r[phase] is not None]
        result[phase]=({k:statistics.median(v[k] for v in values) for k in ['p50','p95','p99']}|{'maximum':max(v['maximum'] for v in values)}) if values else None
    summary.append(result)
(root/'latency-summary.json').write_text(json.dumps(dict(semantic_match=True,rows=rows,summary=summary),indent=2),encoding='utf-8')
for r in summary:
    if r['commands']==64:
        print(r['case'], 'batch p50/p95/p99/max us',*[round_value/1000 for round_value in r['batch'].values()], 'call p50/p99/max us',r['call']['p50']/1000,r['call']['p99']/1000,r['call']['maximum']/1000)

work=[]
for line in (args.work_log or root/'work.log').read_text(encoding='utf-8-sig').splitlines():
    if 'parking-work case=' not in line:continue
    fields={k:int(v) for k,v in re.findall(r'(\w+): (\d+)',line)}
    fields.update({k:int(v) for k,v in re.findall(r'(\w+_bytes)=(\d+)',line)})
    key=f"{fields['active']}/{fields['active']+fields['parked']}/{fields['commands']}/{fields['success_percent']}/{fields['order']}"
    digest=re.search(r'digest=([0-9a-f]+)',line)[1]
    assert digests[key]=={digest}
    assert fields['calls']==fields['commands']
    work.append(dict(case=key,**fields))
assert len(work)==34 and len({w['case'] for w in work})==34

models=[]
model_root=args.models_dir or root
for round in range(1,4):
    for line in (model_root/f'models-{round}.log').read_text(encoding='utf-8-sig').splitlines():
        match=re.search(r'parking-(index|active)-model (.*?) ns=(\[.*\])',line)
        if not match:continue
        fields=dict(re.findall(r'(\w+)=(\w+)',match[2]))
        key=match[1]+':'+','.join(f'{k}={v}' for k,v in fields.items() if k not in ['retained','writes'])
        values=ast.literal_eval(match[3]);assert len(values)==32
        models.append(dict(kind=match[1],case=key,round=round,**fields,**quantiles(values)))
assert len(models)==120
model_summary=[]
for key in dict.fromkeys(r['case'] for r in models):
    group=[r for r in models if r['case']==key]
    assert len(group)==3 and len({(r['retained'],r['writes']) for r in group})==1
    result={k:v for k,v in group[0].items() if k not in ['round','p50','p95','p99','maximum']}
    result.update({k:statistics.median(r[k] for r in group) for k in ['p50','p95','p99']})
    result['maximum']=max(r['maximum'] for r in group)
    model_summary.append(result)
    if result['kind']=='active' or result.get('queries')=='64':
        print(result['case'], 'p50_us',result['p50']/1000,'max_us',result['maximum']/1000,'bytes',result['retained'],'writes',result['writes'])

out=dict(semantic_match=True,measured_batches=len(rows)*16,measured_calls=sum(r['commands']*16 for r in rows),latency=summary,latency_rounds=rows,work=work,models=model_summary,model_rounds=models)
(root/'analysis.json').write_text(json.dumps(out,indent=2),encoding='utf-8')
