# Philosophy
> *Take care of the ideas, and the implementations take care of themselves.*
This is my philosophy on software engineering. It's deliberately divergent from what I think has become the industry
standard way of thinking - and that's intentional. It's not free of bias, and it's shaped entirely by my own experience
and mistakes. If it resonates, great. If not, that's fine too.
## What I Mean by Engineering
Engineering, at its root, is the application of understanding to solve practical problems. A structural engineer
understands material science. An electrical engineer understands circuit theory. The expectation across every traditional
engineering discipline is that you understand the layer below your work - not because it's interesting, but because the
work demands it.
To me, software engineering should carry the same expectation. It's the drive to understand why something works, not
just that it works. That disposition is available at any level of the stack - you can bring it to embedded systems or to
a React frontend. The difference is that lower levels tend to force the question on you. Write firmware and the hardware
will demand you understand it. Write high-level application code and the abstractions are good enough that you can ship
indefinitely without ever asking what's underneath. The engineering mindset is the choice to ask anyway.
I personally think from the ground up, not because I have every layer memorised, but because keeping a working grasp of
how things behave under the hood gives me something to fall back on when things break or the context shifts.
This is the difference between deriving and memorising. When you hold the underlying ideas, rules and best practices
become logical consequences rather than trivia to recall. You don't need a rule for every edge case because the core
concepts generalise.
Not everyone thinks this way, and I don't think they need to. But it's how my brain works, and trying to operate any
other way has only ever slowed me down.
## Simple Ideas, Composed
Beneath every layer of abstraction, computing boils down to simple ideas. By "idea," I mean the smallest indivisible
unit of understanding - not a tool, a pattern, or a framework, but the reason those things exist in the first place.
A pointer is an idea: a memory address referring to another location. Contiguous allocation is an idea: elements stored
side-by-side for efficient hardware access. These aren't opinions or conventions - they're natural consequences of how
the hardware is built.
Complex systems are built by composing these simple ideas. An array is contiguous allocation plus indexed access. A hash
map is an array plus a hashing function. None of it is magic; it's just a stack of well-layered ideas.
We wrap these compositions in labels - patterns, paradigms, architectures - for easy communication. But labels aren't
understanding. Understanding is the ability to tear the label off and see the ideas underneath.
## Moving Forward, Then Looking Back
The industry can't function if every decision gets thought through from first principles. I've learned that the hard
way. You have to move forward, often with incomplete understanding, and that's okay.
One of the most valuable thing I've picked up from people in this industry is the cycle: move forward, learn, iterate,
update. The part that often gets forgotten is the last step - going back to review and update the decisions and
procedures that got you here. Conditions change, assumptions expire, and what worked yesterday might not work tomorrow.
Business considerations are part of this too. Making money isn't the enemy of engineering - it's just another variable.
But like all environment variables, it's constantly updating and needs to be re-evaluated. Treating any constraint as
fixed is where things start to go wrong.
## What C# Taught Me
This philosophy didn't arrive fully formed. A lot of it was forged by wrestling with C#.
C# offers a dozen ways to solve any given problem, but rarely has an opinion on which one you should pick. For someone
trying to reason from first principles, that lack of direction is exhausting. Two senior C# developers can write code
that looks like entirely different languages, because the coherence lives in the programmer, not the tool.
What makes this even more frustrating is that the compiler and JIT are good enough to optimise most of these stylistic
differences away. Different approaches frequently compile down to very similar IL, and the JIT optimises further from
there. So the "rules" about how to write C# aren't really about performance - they're about what the community has
decided is the most human-friendly way to express things. And the human they designed for isn't me.
C# is commonly praised for being readable because it resembles English. But English is full of ambiguity - the same
sentence can mean entirely different things depending on context, tone, and assumption. Modelling a programming language
after that has given me more headaches than I can count. I don't want a language that reads like prose. I want one that
reads like intent.
None of this is to say C# is a bad language - it clearly works brilliantly for a lot of people, and I respect that. But
for me it's a genuine hindrance. I've met other engineers who share that struggle without quite being able to articulate
why, and my current hypothesis is that this might be a big part of it: the language optimises for a kind of readability
that actively works against how some of us think.
That struggle taught me what I actually need: sensible defaults and strong opinions. When a tool has opinions, I can
quickly determine whether it's suitable for my existing problem space - and if it is, I spend less time deciding how to
express something and more time on what I'm expressing. The difference between fighting a tool and flowing with it is
what ultimately pushed me toward this way of thinking.
## Conclusion
Ideas don't change with context. Solutions do. When the ideas are sound, the implementations follow.
This is how I think. It's shaped by my own experience, my own biases, and the handful of people I've been lucky enough
to learn from. I'm not claiming it's right for everyone - just that it's been right for me.
