export type Timestamp = Date;
export type StreamKey = string;
export type SeqNo = bigint;
export type ShardId = bigint;

class SeqPosBeginning { }
class SeqPosEnd { }
class SeqPosAt {
    at: bigint;
    constructor(at: bigint) {
        this.at = at;
    }
}
const SeqPos = {
    Beginning: SeqPosBeginning,
    End: SeqPosEnd,
    At: SeqPosAt,
}
export { SeqPos }
export type SeqPosEnum = SeqPosBeginning | SeqPosEnd | SeqPosAt;

export enum StreamMode {
    /**
     * Streaming from a file at the end
     */
    Live,
    /**
     * Replaying a dead file
     */
    Replay,
    /**
     * Replaying a live file, might catch up to live
     */
    LiveReplay,
}