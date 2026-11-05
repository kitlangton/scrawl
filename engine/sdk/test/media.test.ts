import test from "node:test";
import assert from "node:assert/strict";
import { createMediaJobId, encodeMediaRequest, MediaClient, MediaChildProcessTransport, validateMediaRequest, validateMediaResponse, type DecodeRequest } from "../src/media.ts";
import { ReferenceMediaPeer } from "../src/media-testing.ts";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const req = (times = [1250, 1000, 1250]): DecodeRequest => ({channel:"media",protocolVersion:1,clientId:"1".repeat(32),rpcId:"2".repeat(32),method:"decode",jobId:"3".repeat(32),source:{bundleId:"bundle",bundlePath:"/tmp/bundle.apparent",videoPath:"/tmp/bundle.apparent/recording.mp4",videoOffsetMs:250,identity:{device:"1",inode:"2",size:1000,modifiedNs:"123456789"}},requestedBundleMs:times,outputDirectory:"/tmp/private-job",limits:{maxFrames:48,maxEdge:1280,maxTotalPixels:10_000_000,deadlineMs:30_000}});

test("media wire marker is incompatible with capture v1 and requires explicit source/output/clock", () => {
  assert.throws(() => validateMediaRequest({...req(),channel:"capture"} as never), /channel/);
  assert.throws(() => validateMediaRequest({...req(),protocolVersion:2} as never), /version/);
  assert.throws(() => validateMediaRequest({...req(),source:{...req().source,videoOffsetMs:-1}}), /source/);
  assert.throws(() => validateMediaRequest({...req(),outputDirectory:"relative"}), /decode/);
  assert.throws(() => validateMediaRequest({...req(),requestedBundleMs:[249]}), /precedes/);
  assert.throws(() => validateMediaRequest({...req(),limits:{...req().limits,maxFrames:0}}), /limits/);
  assert.throws(() => validateMediaRequest({...req(),source:{...req().source,identity:{...req().source.identity,inode:"bad"}}}), /identity/);
  assert.throws(() => encodeMediaRequest({...req(),unexpected:true} as never), /fields/);
});

test("test peer is opt-in, carries caller order and duplicates with explicit offset mapping", async () => {
  const peer = new ReferenceMediaPeer();
  const denied = new MediaClient(peer);
  await assert.rejects(denied.handshake(), /explicit test-only opt-in/);
  const client = new MediaClient(peer, {allowTestDouble:true});
  assert.equal((await client.handshake()).implementation, "test-double");
  const result = await client.decode({source:req().source,requestedBundleMs:[1250,1000,1250],outputDirectory:"/tmp/private-job",limits:req().limits});
  assert.deepEqual(result.frames.map(f => [f.requestIndex,f.requestedBundleMs,f.effectiveVideoMs]), [[1,1250,1000],[2,1000,750],[3,1250,1000]]);
  assert.deepEqual(result.frames.map(f => f.path), ["/tmp/private-job/frame-001.png","/tmp/private-job/frame-002.png","/tmp/private-job/frame-003.png"]);
  assert.ok(peer.requests.every(r => r.channel === "media" && r.protocolVersion === 1));
  assert.equal(new Set(peer.requests.map(r => r.rpcId)).size, peer.requests.length);
  await client.close();
});

test("partial decode failure, source mismatch and existing output fail as request errors", async () => {
  for (const scenario of ["partial-decode-failure", "source-mismatch", "output-exists"] as const) {
    const client = new MediaClient(new ReferenceMediaPeer(scenario), {allowTestDouble:true});
    await client.handshake();
    const expected = scenario === "source-mismatch" ? /source_mismatch/ : scenario === "output-exists" ? /invalid_output/ : /decode_failed/;
    await assert.rejects(client.decode({source:req().source,requestedBundleMs:[1000],outputDirectory:"/tmp/private-job",limits:req().limits}), expected);
    await client.close();
  }
});

test("result validation rejects incomplete, reordered and oversized inventories", () => {
  const request = req();
  const base = {channel:"media",protocolVersion:1,rpcId:request.rpcId,result:{jobId:request.jobId,state:"complete",frames:[{requestIndex:1,requestedBundleMs:1250,effectiveVideoMs:1000,clampedToEnd:false,path:"/tmp/private-job/frame-001.png",width:640,height:480,byteLength:128,sha256:"a".repeat(64)},{requestIndex:2,requestedBundleMs:1000,effectiveVideoMs:750,clampedToEnd:false,path:"/tmp/private-job/frame-002.png",width:640,height:480,byteLength:128,sha256:"a".repeat(64)},{requestIndex:3,requestedBundleMs:1250,effectiveVideoMs:1000,clampedToEnd:false,path:"/tmp/private-job/frame-003.png",width:640,height:480,byteLength:128,sha256:"a".repeat(64)}]}};
  assert.equal(validateMediaResponse(base, request.rpcId, request).rpcId, request.rpcId);
  assert.throws(() => validateMediaResponse({...base,result:{...base.result,frames:base.result.frames.slice(0,2)}},request.rpcId,request), /decode result/);
  assert.throws(() => validateMediaResponse({...base,result:{...base.result,frames:[...base.result.frames.slice(1),base.result.frames[0]]}},request.rpcId,request), /reference/);
  const tooLarge = {...base,result:{...base.result,frames:base.result.frames.map(f=>({...f,width:2048,height:2048}))}};
  assert.throws(() => validateMediaResponse(tooLarge,request.rpcId,request), /bounds/);
  const escaped = {...base,result:{...base.result,frames:base.result.frames.map((f,index)=>index === 0 ? {...f,path:"/tmp/private-job-escape/frame.png"} : f)}};
  assert.throws(() => validateMediaResponse(escaped,request.rpcId,request), /escapes/);
});

test("per-request RPC identity, cancel acknowledgment and timeout do not imply completion", async () => {
  const fixture = fileURLToPath(new URL("./media-child-fixture.ts", import.meta.url));
  const transport = new MediaChildProcessTransport(spawn(process.execPath,["--experimental-strip-types",fixture,"timeout"],{stdio:["pipe","pipe","pipe"]}));
  const client = new MediaClient(transport, {allowTestDouble:true});
  await client.handshake();
  const started = Date.now();
  const jobId = createMediaJobId();
  const pending = client.decode({jobId,source:req().source,requestedBundleMs:[1000],outputDirectory:"/tmp/private-job",limits:req().limits},50);
  assert.equal(await client.cancel(jobId), "cancelled");
  await assert.rejects(pending, /timed out/);
  assert.ok(Date.now() - started < 500);
  const cancelPeer = new ReferenceMediaPeer();
  const cancelClient = new MediaClient(cancelPeer,{allowTestDouble:true});
  await cancelClient.handshake();
  assert.equal(await cancelClient.cancel("4".repeat(32)), "cancelled");
  assert.equal((cancelPeer.requests.at(-1) as {rpcId:string}).rpcId.length,32);
  await client.close(); await cancelClient.close();
});
