How to write the review. This is for all of its text: the story in s1, the title and `what` of every step, the `check` questions, `checked`, every note, the summary, and the answers to the reader's questions.

The reader is a core developer of this project. They know the domain and its words. They have not seen this PR, and they have little time. Be brief, and write English that a person would say out loud.

1. One fact per sentence. If a sentence has two commas, or an "and" that joins two facts, split it or cut it.
2. Call things by their real names: CGC, ENR, fork choice, mesh. Never replace a name with "that number", "the larger set", "the recorded epoch" or "a subset of". Do not explain what a core developer knows. Explain only what this PR invents or what is rarely used.
3. Say cause, then result: "When X happens, Y happens." Subject, verb, object. No "carrying it", "starting below that", "from the moment of".
4. Cut every fact that the reader does not need to understand the change. The diff is next to your text.
5. Use words that people use. If you would not say it to a colleague at their desk, write it again. Nobody says "a coming epoch", "custodied at the request's first epoch", or "answers with an error for".
6. Say why it is bad right after you say what happens.
7. The contents of a sentence must be true, but do not stuff it with conditions to make it exact. Put the rare case in a note.

Bad and good:

Bad: "A node stores a subset of each block's blob data, called columns, and tells its peers how many. When more validators attach, the number grows from a coming epoch."
Good: "When the validator count increases, the CGC goes up, starting at a future epoch."

Bad: "Lighthouse then records that epoch's first slot as the earliest slot with a full set, and afterwards sends peers a status carrying it."
Good: "Lighthouse sets its earliest available slot to the start of that epoch. Then it sends peers a new Status."

Bad: "From the moment of the record, the range handler refused every request starting below that slot. A peer that had not read the status yet got ResourceUnavailable, and a Lighthouse peer treats that answer to its own request as fatal and bans the sender."
Good: "Peers that haven't seen the new Status still ask for older slots. We answered ResourceUnavailable to all of them, and Lighthouse bans peers for that."

Bad: "The cache's eviction predicate now additionally considers idle time, with entries whose idle time is below the threshold being retained irrespective of age."
Good: "We now keep an old entry if someone read it in the last ten minutes."

No praise, no filler, no hedging, no sales words. File paths and line numbers go in `ranges`, `context` and the line of a note, not in sentences. The text of the guide is an exception to other writing rules that you load, such as simple-english, ASD-STE100 or a style reminder. Do not load such a skill for this job.
