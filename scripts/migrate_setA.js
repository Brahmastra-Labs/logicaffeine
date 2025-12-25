#!/usr/bin/env node
/**
 * Migrate Logicaffeine Set A to LOGOS curriculum JSON format
 * Generates explanations for all questions
 */

const fs = require('fs');
const path = require('path');

const sourceFile = path.join(__dirname, '../../external-resources/logicola/content/sets/setA.ts');
const content = fs.readFileSync(sourceFile, 'utf8');

const outDir = path.join(__dirname, '../assets/curriculum/00_logicaffeine/01_syllogistic');

// Template variable expansions from constants.ts
const TEMPLATES = {
    feedback_single_person: 'stands for a single person, and so translates into a small letter.'
};

// Clear existing exercise files
const existingFiles = fs.readdirSync(outDir).filter(f => f.startsWith('ex_'));
existingFiles.forEach(f => fs.unlinkSync(path.join(outDir, f)));

const questions = [];

// Split into lines and parse manually
const lines = content.split('\n');
let inQuestion = false;
let currentQuestion = null;
let inOptions = false;
let currentOptions = [];

for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Detect start of a question block
    const idMatch = line.match(/id:\s*['"](\d+\.\d+)['"]/);
    if (idMatch && !inQuestion) {
        inQuestion = true;
        currentQuestion = { id: idMatch[1], options: [], explanation: null };
        continue;
    }

    if (inQuestion) {
        // Parse prompt
        const promptMatch = line.match(/prompt:\s*[`'"](.+?)[`'"]\s*,?$/);
        if (promptMatch) {
            currentQuestion.prompt = promptMatch[1].replace(/\\'/g, "'").replace(/\\"/g, '"');
        }

        // Detect options array start
        if (line.includes('options:') && line.includes('[')) {
            inOptions = true;
            currentOptions = [];
        }

        // Parse option labels
        if (inOptions) {
            const labelMatch = line.match(/\{\s*id:\s*(\d+)\s*,\s*label:\s*['"]([^'"]+)['"]/);
            if (labelMatch) {
                currentOptions[parseInt(labelMatch[1])] = labelMatch[2];
            }

            // End of options array
            if (line.includes('],')) {
                inOptions = false;
                currentQuestion.options = currentOptions.filter(o => o !== undefined);
            }
        }

        // Parse correctId
        const correctMatch = line.match(/correctId:\s*\[([^\]]+)\]/);
        if (correctMatch) {
            currentQuestion.correctIds = correctMatch[1].split(',').map(s => parseInt(s.trim()));
        }

        // Parse answer/explanation - handle multiple formats
        // Format 1: answer: 'string',
        // Format 2: answer: "string",
        // Format 3: answer: `template ${var}`,
        // Format 4: answer: '',
        if (line.includes('answer:')) {
            // Check for template literal with variable
            const templateMatch = line.match(/answer:\s*`([^`]*)\$\{(\w+)\}[^`]*`/);
            if (templateMatch) {
                const prefix = templateMatch[1];
                const varName = templateMatch[2];
                const expansion = TEMPLATES[varName] || '';
                currentQuestion.explanation = (prefix + expansion).trim();
            } else {
                // Regular string
                const stringMatch = line.match(/answer:\s*['"]([^'"]*)['"]/);
                if (stringMatch) {
                    currentQuestion.explanation = stringMatch[1] || null;
                }
            }
        }

        // Detect end of question block
        if (line.trim() === '},') {
            if (currentQuestion.id && currentQuestion.prompt && currentQuestion.options.length > 0 && currentQuestion.correctIds) {
                questions.push({
                    id: `A_${currentQuestion.id}`,
                    prompt: currentQuestion.prompt,
                    options: currentQuestion.options,
                    correct: currentQuestion.correctIds.length === 1 ? currentQuestion.correctIds[0] : currentQuestion.correctIds,
                    explanation: currentQuestion.explanation
                });
            }
            inQuestion = false;
            currentQuestion = null;
        }
    }
}

console.log(`Parsed ${questions.length} questions`);

/**
 * Generate an explanation for a syllogistic logic question
 */
function generateExplanation(prompt, options, correctIndex) {
    const correctAnswer = options[correctIndex];
    if (!correctAnswer) return "Select the correct logical translation.";

    // Analyze the correct answer pattern
    const hasAll = correctAnswer.startsWith('all ') || correctAnswer.startsWith('All ');
    const hasSome = correctAnswer.startsWith('some ') || correctAnswer.startsWith('Some ');
    const hasNo = correctAnswer.startsWith('no ') || correctAnswer.startsWith('No ');
    const hasNot = correctAnswer.includes(' not ') || correctAnswer.includes(' is not ');

    // Check case patterns in the answer
    const parts = correctAnswer.replace(/^(all |some |no )/i, '').split(' is ');
    if (parts.length !== 2) {
        return "Translate the English sentence into syllogistic notation.";
    }

    const subject = parts[0].trim();
    const predicate = parts[1].replace('not ', '').trim();

    const subjectLower = subject === subject.toLowerCase();
    const predicateLower = predicate === predicate.toLowerCase();

    let explanation = '';

    // Explain the subject
    if (subjectLower) {
        if (subject === 'i') {
            explanation += '"I" refers to a specific individual (the speaker), so use lowercase "i". ';
        } else if (subject === 'u') {
            explanation += '"You" refers to a specific individual (the listener), so use lowercase "u". ';
        } else {
            explanation += `"${subject}" refers to a specific individual, so use lowercase. `;
        }
    } else {
        explanation += `"${subject}" refers to a class of things, so use uppercase. `;
    }

    // Explain the predicate
    if (predicateLower) {
        explanation += `The predicate "${predicate}" refers to a specific individual (like "the X-est Y"), so use lowercase.`;
    } else {
        explanation += `The predicate "${predicate}" refers to a class/property, so use uppercase.`;
    }

    // Add quantifier explanation if present
    if (hasAll) {
        explanation = `"All" indicates a universal statement about every member of a class. ` + explanation;
    } else if (hasSome) {
        explanation = `"Some" indicates an existential statement about at least one member. ` + explanation;
    } else if (hasNo) {
        explanation = `"No" indicates a universal negative - no members of the class have the property. ` + explanation;
    }

    // Add negation note
    if (hasNot) {
        explanation += ` The "not" applies to the predication, indicating the subject lacks the property.`;
    }

    return explanation;
}

// Write each question as a separate JSON file
let withExplanation = 0;
let generated = 0;

questions.forEach((q, i) => {
    const paddedIndex = String(i + 1).padStart(3, '0');
    const filename = `ex_${paddedIndex}.json`;
    const filepath = path.join(outDir, filename);

    let explanation = q.explanation;
    if (!explanation || explanation.trim() === '') {
        const correctIdx = Array.isArray(q.correct) ? q.correct[0] : q.correct;
        explanation = generateExplanation(q.prompt, q.options, correctIdx);
        generated++;
    } else {
        withExplanation++;
    }

    const exercise = {
        id: q.id,
        type: "multiple_choice",
        difficulty: 1,
        prompt: q.prompt,
        options: q.options,
        correct: Array.isArray(q.correct) ? q.correct[0] : q.correct,
        hint: null,
        explanation: explanation
    };

    fs.writeFileSync(filepath, JSON.stringify(exercise, null, 2) + '\n');
});

console.log(`Wrote ${questions.length} exercise files to ${outDir}`);
console.log(`  - ${withExplanation} had original explanations`);
console.log(`  - ${generated} had explanations generated`);

// Verify samples
console.log('\n=== Sample with original explanation ===');
const sample1 = JSON.parse(fs.readFileSync(path.join(outDir, 'ex_007.json'), 'utf8'));
console.log(JSON.stringify(sample1, null, 2));

console.log('\n=== Sample with generated explanation ===');
const sample2 = JSON.parse(fs.readFileSync(path.join(outDir, 'ex_001.json'), 'utf8'));
console.log(JSON.stringify(sample2, null, 2));
