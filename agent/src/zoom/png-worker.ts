import { parentPort } from 'node:worker_threads';
import { PNG } from 'pngjs';

if (!parentPort) throw new Error('PNG worker requires a parent port');

type Request = { id: number; bytes: ArrayBuffer; crop: [number,number,number,number]; maxWidth: number; maxHeight: number };
parentPort.on('message', (request: Request) => {
  try {
    const source = PNG.sync.read(Buffer.from(request.bytes), { checkCRC: true });
    const [x0,y0,x1,y1] = request.crop;
    const left=Math.max(0,Math.min(source.width-1,Math.floor(x0*source.width)));
    const top=Math.max(0,Math.min(source.height-1,Math.floor(y0*source.height)));
    const right=Math.max(left+1,Math.min(source.width,Math.ceil(x1*source.width)));
    const bottom=Math.max(top+1,Math.min(source.height,Math.ceil(y1*source.height)));
    const cropWidth=right-left,cropHeight=bottom-top;
    const scale=Math.min(1,request.maxWidth/cropWidth,request.maxHeight/cropHeight);
    const width=Math.max(1,Math.floor(cropWidth*scale)),height=Math.max(1,Math.floor(cropHeight*scale));
    const data=Buffer.alloc(width*height*4);
    for(let y=0;y<height;y++) for(let x=0;x<width;x++) {
      const sx=left+Math.min(cropWidth-1,Math.floor(x/scale)),sy=top+Math.min(cropHeight-1,Math.floor(y/scale));
      const si=(sy*source.width+sx)*4,di=(y*width+x)*4;
      source.data.copy(data,di,si,si+4);
    }
    const transferable=data.buffer.slice(data.byteOffset,data.byteOffset+data.byteLength);
    parentPort!.postMessage({id:request.id,width,height,data:transferable},[transferable]);
  } catch(error) {
    parentPort!.postMessage({id:request.id,error:error instanceof Error?error.message:'PNG decode failed'});
  }
});
