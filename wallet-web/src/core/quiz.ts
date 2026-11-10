/** 1-based positions. 24-word phrases use 3, 8, and 14. Shorter phrases stay inside the list. */
export function quizPositions(count: number): number[] {
  if (count >= 14) return [3, 8, 14];
  if (count >= 12) return [3, 8, 12];
  return [1, Math.min(2, count), count].filter((value, index, all) => all.indexOf(value) === index);
}

export function quizChoices(words: string[], position: number): string[] {
  const correct = words[position - 1] ?? "";
  const others = words.filter((word, index) => index !== position - 1 && word !== correct);
  const picked = [correct];
  for (const word of others) {
    if (picked.length >= 4) break;
    picked.push(word);
  }
  return picked.sort((a, b) => a.localeCompare(b));
}

export function quizMatches(words: string[], answers: Record<number, string>): boolean {
  return quizPositions(words.length).every((position) => {
    const expected = words[position - 1] ?? "";
    return expected.length > 0 && (answers[position] ?? "").trim().toLowerCase() === expected.toLowerCase();
  });
}
