// Original, deterministic two-frame test artwork; no image package is required.
import { mkdir, writeFile } from 'node:fs/promises';
const directory = new URL('../examples/media-resources/', import.meta.url);
await mkdir(directory,{recursive:true});
const parts=[Buffer.from('GIF89a'),Buffer.from([16,0,16,0,0xf1,0,0]),Buffer.from([0,0,0,47,113,87,227,171,83,25,40,30]),Buffer.from([0x21,0xff,11]),Buffer.from('NETSCAPE2.0'),Buffer.from([3,1,0,0,0])];
for(let frame=0;frame<2;frame++) {
  const pixels=[];
  for(let y=0;y<16;y++)for(let x=0;x<16;x++) {
    let color=(x>=2&&x<=13&&y>=4&&y<=13)||(y>=1&&y<=4&&((x>=2&&x<=5)||(x>=10&&x<=13)))?1:0;
    if((x===5||x===10)&&y===(frame?8:7))color=3;
    if((x===7||x===8)&&y===10)color=2;
    pixels.push(color);
  }
  // Repeated clear codes keep the LZW width at three bits, deliberately simple
  // and independent of any platform encoder. The test fixture is only 834 B.
  const codes=pixels.flatMap(p=>[4,p]);codes.push(5);
  let acc=0,bits=0;const encoded=[];
  for(const code of codes){acc|=code<<bits;bits+=3;while(bits>=8){encoded.push(acc&255);acc>>=8;bits-=8;}}
  if(bits)encoded.push(acc&255);
  parts.push(Buffer.from([0x21,0xf9,4,9,frame?24:8,0,0,0]),Buffer.from([0x2c,0,0,0,0,16,0,16,0,0,2]));
  for(let at=0;at<encoded.length;at+=255){const block=encoded.slice(at,at+255);parts.push(Buffer.from([block.length,...block]));}parts.push(Buffer.from([0]));
}
parts.push(Buffer.from([0x3b]));
await writeFile(new URL('blink.gif',directory),Buffer.concat(parts));
await writeFile(new URL('LICENSE.txt',directory),'Original Desktop Buddy Studio GIF test artwork.\nSPDX-License-Identifier: MIT\nCopyright (c) 2026 Desktop Buddy Studio contributors.\n');
