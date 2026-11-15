"""未知存档的轻量文本线索：复用有界解压，只提供原字节候选，不识别活动状态。"""
from pathlib import Path
import re
import runpy

CODECS=runpy.run_path(str(Path(__file__).with_name('bytecodecs.py')))


def text_candidates(data):
    result=[];seen=set()
    # 不对每个字节重新解码整份文件；按引号定位短片段，并严格回解原始字节。
    for encoding in ('utf-8','utf-16-le','utf-16-be','cp932','gb18030'):
        count=0
        for left,right in (('「','」'),('『','』'),('“','”'),('"','"')):
            opening,closing=left.encode(encoding),right.encode(encoding)
            cursor=0
            while count<24:
                start=data.find(opening,cursor)
                if start<0:break
                end=data.find(closing,start+len(opening),start+4096)
                cursor=start+len(opening)
                if end<0:continue
                raw=data[start:end+len(closing)]
                try:text=raw.decode(encoding)
                except UnicodeError:continue
                if len(text)<6 or not all(c.isprintable() or c in '\r\n\t' for c in text):continue
                key=(start,len(raw),text)
                if key in seen:continue
                seen.add(key);count+=1
                result.append(dict(offset=start,length=len(raw),encoding=encoding,text=text))
    return result


def probe(data):
    """候选偏移属于各自视图；只接受完整有界压缩流，失败不阻断自写解析。"""
    views=[dict(codec='raw',compressed_offset=0,data=data)]
    attempts=0
    for offset in range(min(4096,max(0,len(data)-1))):
        head=data[offset:offset+6]
        codec=('zlib' if head[0]&15==8 and head[0]>>4<=7 and int.from_bytes(head[:2],'big')%31==0
               else 'bz2' if head.startswith(b'BZh') else 'lzma' if head.startswith(b'\xfd7zXZ\x00') else None)
        if not codec:continue
        attempts+=1
        try:body=CODECS['decode'](codec,data[offset:])
        except ValueError:body=None
        if body is not None:
            views.append(dict(codec=codec,compressed_offset=offset,data=body))
            break
        if attempts>=16:break
    for view in views:
        body=view.pop('data')
        view.update(length=len(body),text_candidates=text_candidates(body),identifiers=[
            dict(offset=m.start(),text=m[1].decode('ascii')) for m in
            list(re.finditer(rb'(?<![\x20-\x7e])([A-Za-z0-9][A-Za-z0-9_./-]{3,95})\x00',body))[:64]])
    return dict(views=views,scope='Quoted text and ASCII identifiers in raw/complete decompressed views; offsets belong to that view. All are candidates, possibly history/cache, never active-state proof.')
