export type ParsedPoint = {
  x: number;
  y: number;
  label: string;
  screenIndex: number;
  raw: string;
};

const pointTagPattern = /\[POINT:(\d+),(\d+):([^:\]]+):(\d+)\]/g;

export function extractPointTags(assistantText: string): ParsedPoint[] {
  const found: ParsedPoint[] = [];
  let matchCaptured: RegExpExecArray | null;
  pointTagPattern.lastIndex = 0;
  while ((matchCaptured = pointTagPattern.exec(assistantText)) !== null) {
    found.push({
      x: Number(matchCaptured[1]),
      y: Number(matchCaptured[2]),
      label: matchCaptured[3]?.trim() ?? "",
      screenIndex: Number(matchCaptured[4]),
      raw: matchCaptured[0],
    });
  }
  pointTagPattern.lastIndex = 0;
  return found;
}

export function stripPointTagsForSpeech(assistantText: string): string {
  pointTagPattern.lastIndex = 0;
  return assistantText.replace(pointTagPattern, "").trim();
}
