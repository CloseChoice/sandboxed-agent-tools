#!/usr/bin/env python3
"""Exercise the actual WASI XML parsers and host filesystem grants."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT=Path(__file__).resolve().parent
WRAPPER=Path.home()/'.codex/skills/sat/scripts/run_sat.py'

CODE=r'''
import io
import xml.etree.ElementTree as ET
import xml.etree.cElementTree as CET
from xml.etree import ElementInclude
import xml.dom.minidom as DOM
import xml.dom.pulldom as pulldom
from xml.dom.xmlbuilder import DOMBuilder, DOMInputSource
import xml.sax
from xml.sax.handler import ContentHandler
from xml.sax.saxutils import XMLGenerator
from xml.parsers import expat

source='<root xmlns:x="urn:test"><x:item id="1">Grüße &amp; 世界</x:item></root>'
root=ET.fromstring(source)
assert root.find('x:item',{'x':'urn:test'}).text=='Grüße & 世界'
assert ET.fromstring(ET.tostring(root)).find('{urn:test}item').get('id')=='1'
for encoding in ['us-ascii','utf-8','utf-16','iso-8859-1']:
    assert ET.fromstring(ET.tostring(root,encoding=encoding)).find('{urn:test}item').text=='Grüße & 世界'
assert CET.fromstring('<root/>').tag=='root'
included=ET.fromstring('<root xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="child.xml"/></root>')
ElementInclude.include(included,loader=lambda href,parse,encoding=None:ET.fromstring('<child>included</child>'))
assert included.find('child').text=='included'
assert [e.tag for event,e in ET.iterparse(io.StringIO(source),events=('end',))]==['{urn:test}item','root']
assert 'Grüße' in ET.canonicalize(source)
parser=ET.XMLPullParser(events=('end',))
parser.feed('<root>');parser.feed('<item>incremental</item></root>')
assert [e.tag for event,e in parser.read_events()]==['item','root']
parser.close()
try:ET.fromstring('<root>')
except ET.ParseError:pass
else:raise AssertionError('malformed XML accepted')

doc=DOM.parseString(source)
assert doc.getElementsByTagNameNS('urn:test','item')[0].firstChild.data=='Grüße & 世界'
assert DOM.parseString(doc.toxml()).documentElement.tagName=='root'
doc.unlink()
dom_input=DOMInputSource();dom_input.byteStream=io.BytesIO(b'<root>builder</root>')
assert DOMBuilder().parse(dom_input).documentElement.firstChild.data=='builder'
events=pulldom.parseString('<root><item>pull DOM</item></root>')
for event,node in events:
    if event==pulldom.START_ELEMENT and node.tagName=='item':
        events.expandNode(node)
        assert node.firstChild.data=='pull DOM'

class Handler(ContentHandler):
    def __init__(self):super().__init__();self.text=[]
    def characters(self,data):self.text.append(data)
handler=Handler();xml.sax.parseString(source,handler)
assert 'Grüße & 世界' in ''.join(handler.text)
output=io.StringIO();writer=XMLGenerator(output,encoding='utf-8')
writer.startDocument();writer.startElement('root',{});writer.characters('a & b');writer.endElement('root');writer.endDocument()
assert ET.fromstring(output.getvalue()).text=='a & b'
seen=[];parser=expat.ParserCreate();parser.StartElementHandler=lambda name,attrs:seen.append(name)
parser.Parse('<root><item/></root>',True)
assert seen==['root','item']
print('XML APIs passed')
'''

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sat',type=Path,required=True)
    args=parser.parse_args()
    env=dict(os.environ,SAT_BINARY=str(args.sat.resolve()))
    prefix=[sys.executable,str(WRAPPER),'--'] if WRAPPER.exists() else [str(args.sat.resolve())]
    def run(code,grants=(),ok=True,allow_secret=False):
        result=subprocess.run([*prefix,'python','-c',code,*(['--',*map(str,grants)] if grants else [])],
                              env=env,capture_output=True,timeout=180)
        assert (result.returncode==0)==ok,('XML call',result.returncode,result.stderr.decode(errors='replace')[-3000:])
        if not ok:
            assert b'ModuleNotFoundError' not in result.stderr,result.stderr.decode(errors='replace')[-3000:]
            assert b'LookupError: unknown encoding' not in result.stderr,result.stderr.decode(errors='replace')[-3000:]
        if not allow_secret:assert b'UNGRANTED-XML-SECRET' not in result.stdout+result.stderr
        return result.stdout.decode()
    assert 'XML APIs passed' in run(CODE)
    with tempfile.TemporaryDirectory(prefix='sat-xml-') as tmp:
        root=Path(tmp)
        source=root/'input.xml';source.write_text('<root><item>granted</item></root>')
        secret=root/'secret.xml';secret.write_text('<root>UNGRANTED-XML-SECRET</root>')
        output=root/'output.xml';output.touch()
        parse=f'import xml.etree.ElementTree as E; print(E.parse({str(source)!r}).getroot().find("item").text)'
        run(parse,ok=False)
        assert run(parse,['--read',source]).strip()=='granted'
        write=f'import xml.etree.ElementTree as E; E.ElementTree(E.Element("written")).write({str(output)!r},encoding="utf-8")'
        run(write,['--read',output],ok=False)
        assert output.read_bytes()==b''
        run(write,['--write',output])
        assert output.read_bytes()==b'<written />'
        # Exercise parser file input too, not just ElementTree's open wrapper.
        assert 'root' in run(f'import xml.dom.minidom as D; print(D.parse({str(source)!r}).documentElement.tagName)',['--read',source])
        run(f'import xml.dom.minidom as D; D.parse({str(secret)!r})',['--read',source],ok=False)
        # Explicit external-entity requests still cannot read an ungranted file.
        document=f'<!DOCTYPE root [<!ENTITY leak SYSTEM "{secret.as_uri()}">]><root>&leak;</root>'
        code='''import io,xml.sax
from xml.sax.handler import ContentHandler,feature_external_ges
class Handler(ContentHandler):
    def characters(self,s):print(s)
p=xml.sax.make_parser();p.setFeature(feature_external_ges,True);p.setContentHandler(Handler())
p.parse(io.StringIO('''+repr(document)+'''))'''
        run(code,['--read',source],ok=False)
        assert 'UNGRANTED-XML-SECRET' in run(code,['--read',secret],allow_secret=True)
    print('XML parsing, writing, DOM/SAX/Expat and denied filesystem access passed')

if __name__=='__main__':main()
