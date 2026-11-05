const mode = process.argv[2];
let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", async (chunk) => {
  buffer += chunk;
  while (buffer.includes("\n")) {
    const index = buffer.indexOf("\n");
    const request = JSON.parse(buffer.slice(0, index)) as {rpcId:string;method:string;jobId:string};
    buffer = buffer.slice(index + 1);
    if (request.method === "decode" && mode === "timeout") continue;
    const result = request.method === "handshake"
      ? {channel:"media",protocolVersion:1,implementation:"test-double",capabilities:{decodeFrames:false}}
      : {jobId:request.jobId,state:"cancelled"};
    process.stdout.write(JSON.stringify({channel:"media",protocolVersion:1,rpcId:request.rpcId,result}) + "\n");
  }
});
process.stdin.on("end", () => process.exit(0));
