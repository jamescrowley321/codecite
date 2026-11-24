"""Offline tests for the #287 runner: python3 -m unittest experiments/retrieval-generalization/test_run.py"""

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run  # noqa: E402

PIN = 'a' * 40
CORPORA = {
    'alpha': dict(name='alpha', split='development', commit=PIN, url='https://example.invalid/alpha'),
    'beta': dict(name='beta', split='evaluation', commit='b' * 40, url='https://example.invalid/beta'),
}
SOURCE = {('alpha', 'src/lib.rs'): ['// header', 'pub fn parse_header() {}', '}'],
          ('beta', 'app/main.py'): ['class Request:', '    def send(self):', '        pass']}


def case(**over):
    base = dict(id='alpha-exact-1', repo='alpha', revision=PIN, query='parse_header', path=None,
                answers=[dict(path='src/lib.rs', line=2)], score=True, label_source='test')
    base['class'] = over.pop('cls', 'exact')
    base.update(over)
    return base


def read_line(repo, path, line):
    lines = SOURCE.get((repo, path))
    return lines[line - 1] if lines and 1 <= line <= len(lines) else None


def heads(repo):
    return CORPORA[repo]['commit']


class ValidateTest(unittest.TestCase):
    def errors(self, cases, corpora=CORPORA, head=heads):
        return run.validate(corpora, cases, head, read_line)

    def test_a_good_workload_passes(self):
        good = [case(), case(id='beta-q', repo='beta', revision='b' * 40, cls='qualified', query='Request.send',
                             answers=[dict(path='app/main.py', line=2)]),
                case(id='alpha-none', cls='no-result', query='zzz', answers=[])]
        self.assertEqual(self.errors(good), [])

    def test_duplicate_ids_fail(self):
        self.assertTrue(any('duplicate' in e for e in self.errors([case(), case()])))

    def test_a_revision_other_than_the_pin_fails(self):
        self.assertTrue(any('pin' in e for e in self.errors([case(revision='c' * 40)])))

    def test_a_checkout_not_at_the_pin_fails(self):
        self.assertTrue(any('checkout' in e for e in self.errors([case()], head=lambda r: 'd' * 40)))

    def test_a_missing_source_line_fails(self):
        self.assertTrue(any('no line' in e for e in self.errors([case(answers=[dict(path='src/lib.rs', line=9)])])))
        self.assertTrue(any('no line' in e for e in self.errors([case(answers=[dict(path='nope.rs', line=1)])])))

    def test_a_definition_label_must_name_the_symbol_on_its_line(self):
        self.assertTrue(any('does not name' in e for e in self.errors([case(answers=[dict(path='src/lib.rs', line=1)])])))

    def test_a_repository_in_both_splits_fails(self):
        corpora = dict(CORPORA, gamma=dict(name='gamma', split='evaluation', commit=PIN,
                                           url='https://example.invalid/alpha'))
        self.assertTrue(any('both splits' in e for e in self.errors([case()], corpora=corpora)))

    def test_an_unknown_split_fails(self):
        corpora = dict(CORPORA, alpha=dict(CORPORA['alpha'], split='tuning'))
        self.assertTrue(any('split' in e for e in self.errors([case()], corpora=corpora)))

    def test_a_no_result_case_with_answers_fails(self):
        self.assertTrue(any('no-result' in e for e in self.errors([case(cls='no-result')])))

    def test_a_scored_positive_case_needs_answers(self):
        self.assertTrue(any('needs answers' in e for e in self.errors([case(answers=[])])))

    def test_a_path_case_needs_a_filter(self):
        self.assertTrue(any('path filter' in e for e in self.errors([case(cls='path')])))

    def test_main_refuses_before_spawning(self):
        spawned = []
        with self.assertRaises(SystemExit):
            run.check_or_exit(CORPORA, [case(), case()], heads, read_line)
        self.assertEqual(spawned, [])


class ParseTest(unittest.TestCase):
    def test_codecite_search(self):
        out = json.dumps({'results': [{'citation': 'alpha@' + PIN + ':src/lib.rs#L2-L3'},
                                      {'citation': 'alpha@' + PIN + ':a:b.rs#L7'}]})
        status, rows, total = run.parse('codecite', 'search', 0, out, '')
        self.assertEqual((status, rows, total), ('ok', [('src/lib.rs', 2, 3), ('a:b.rs', 7, 7)], 2))

    def test_codecite_empty_search_is_empty_not_error(self):
        self.assertEqual(run.parse('codecite', 'search', 0, '{"results":[]}', '')[0], 'empty')

    def test_codecite_def_not_found_is_empty(self):
        err = 'Error: the corpus defines no symbol `x`; no candidates\n'
        self.assertEqual(run.parse('codecite', 'def', 1, '', err)[0], 'empty')

    def test_codecite_failure_is_error(self):
        self.assertEqual(run.parse('codecite', 'search', 1, '', 'Error: no corpus')[0], 'error')
        self.assertEqual(run.parse('codecite', 'search', 0, 'not json', '')[0], 'error')

    def test_codecite_def_total(self):
        out = json.dumps({'symbol': 'x', 'ambiguous': True, 'total': 70,
                          'results': [{'citation': 'alpha@' + PIN + ':src/lib.rs#L2-L3'}]})
        self.assertEqual(run.parse('codecite', 'def', 0, out, ''), ('ok', [('src/lib.rs', 2, 3)], 70))

    def test_codegraph_query(self):
        out = json.dumps([{'node': {'filePath': 'app/main.py', 'startLine': 2, 'endLine': 3}, 'score': 1.0}])
        self.assertEqual(run.parse('codegraph', 'query', 0, out, ''), ('ok', [('app/main.py', 2, 3)], 1))
        self.assertEqual(run.parse('codegraph', 'query', 0, '[]', '')[0], 'empty')

    def test_codegraph_node(self):
        out = ('**send** (method)\n\n**Location:** app/main.py:2\n\n```python\n2\t    def send(self):\n'
               '3\t        pass\n```\n')
        self.assertEqual(run.parse('codegraph', 'node', 0, out, ''), ('ok', [('app/main.py', 2, 3)], 1))
        out = '**Request** (class)\n\n**Location:** app/main.py:1\n\n**Members (1):**\n'
        self.assertEqual(run.parse('codegraph', 'node', 0, out, '')[1], [('app/main.py', 1, 1)])
        self.assertEqual(run.parse('codegraph', 'node', 0, 'Symbol "x" not found in the codebase\n', '')[0], 'empty')
        self.assertEqual(run.parse('codegraph', 'node', 0, 'something odd\n', '')[0], 'error')

    def test_first_hit_matches_path_and_span_not_names(self):
        answers = [dict(path='src/lib.rs', line=2)]
        self.assertEqual(run.first_hit([('src/other.rs', 1, 9), ('src/lib.rs', 1, 3)], answers), 2)
        self.assertIsNone(run.first_hit([('src/lib.rs', 3, 9)], answers))
        self.assertIsNone(run.first_hit([('lib.rs', 1, 9)], answers))


HELPER = '''total\t120
top\t1\tsrc/other.rs\t1\t9\tfunction\tsource\tother\t5.0
target\tsrc/lib.rs:2\t{rank}\t1\t3\tfunction\tsource\tparse_header\t1.0
doc\tsrc/lib.rs:2\t{present}\trust\tsource\t{chunks}\tfunction:parse_header\t{symbols}
'''


def helper(rank=0, present=1, chunks=1, symbols='parse_header|function|2-3'):
    return run.parse_candidates(HELPER.format(rank=rank, present=present, chunks=chunks, symbols=symbols))


class ClassifyTest(unittest.TestCase):
    def outcome(self, rank, status='ok', diag=None, tool='codecite', op='search', cls='exact', observed=None):
        return run.classify(case(cls=cls), tool, op, status, rank, diag,
                            observed if observed is not None else [('src/other.rs', 1, 9)])

    def test_target_in_output(self):
        self.assertEqual(self.outcome(1), 'hit')
        self.assertEqual(self.outcome(4), 'rank:in-output')

    def test_target_absent_but_in_the_candidate_set(self):
        self.assertEqual(self.outcome(None, diag=helper(rank=57)), 'rank:below-cut')

    def test_target_indexed_but_not_a_candidate(self):
        self.assertEqual(self.outcome(None, diag=helper(rank=0)), 'candidate:not-matched')

    def test_target_absent_from_the_index(self):
        self.assertEqual(self.outcome(None, diag=helper(present=0, chunks=0, symbols='')), 'extraction:not-indexed')
        self.assertEqual(self.outcome(None, diag=helper(chunks=0, symbols='')), 'extraction:not-indexed')

    def test_unresolved_without_diagnosis_or_when_the_helper_disagrees_with_the_cli(self):
        self.assertEqual(self.outcome(None, diag=None), 'unresolved')
        self.assertEqual(self.outcome(None, diag=helper(rank=57), observed=[('src/x.rs', 1, 2)]), 'unresolved')
        self.assertEqual(self.outcome(None, diag={'error': 'boom'}), 'unresolved')

    def test_never_infers_absence_from_a_capped_list(self):
        # 50 observed results, none the target, and no diagnosis: not "not a candidate".
        self.assertEqual(self.outcome(None, diag=None, observed=[('x.rs', i, i) for i in range(50)]), 'unresolved')

    def test_unsupported_error_and_true_empty_are_distinct(self):
        self.assertEqual(self.outcome(None, status='unsupported'), 'unsupported')
        self.assertEqual(self.outcome(None, status='error'), 'error')
        self.assertEqual(run.classify(case(cls='no-result', answers=[]), 'codecite', 'search', 'empty', None, None, []),
                         'correct-empty')
        self.assertEqual(run.classify(case(cls='no-result', answers=[]), 'codecite', 'search', 'ok', None, None,
                                      [('a', 1, 1)]), 'false-positive')
        # An empty answer to a positive case is a miss, diagnosed like any other.
        none = run.parse_candidates('total\t0\ntarget\tsrc/lib.rs:2\t0\t-\t-\t-\t-\t-\t-\n'
                                    'doc\tsrc/lib.rs:2\t1\trust\tsource\t1\tfunction:parse_header\tparse_header|function|2-3\n')
        self.assertEqual(self.outcome(None, status='empty', diag=none, observed=[]), 'candidate:not-matched')
        # An empty CLI answer while the helper finds candidates is a scope disagreement.
        self.assertEqual(self.outcome(None, status='empty', diag=helper(rank=0), observed=[]), 'unresolved')

    def test_definition_lookup_misses(self):
        d = dict(helper(symbols=''), deftotal=0)
        self.assertEqual(self.outcome(None, op='def', diag=d, observed=[]), 'extraction:not-a-definition')
        d = dict(helper(), deftotal=80)
        self.assertEqual(self.outcome(None, op='def', diag=d), 'rank:below-cut')
        d = dict(helper(), deftotal=1)
        self.assertEqual(self.outcome(None, op='def', diag=d), 'candidate:lookup-miss')

    def test_codegraph_misses_use_its_node_table(self):
        self.assertEqual(self.outcome(None, tool='codegraph', op='query', diag={'nodes': []}),
                         'extraction:not-a-definition')
        self.assertEqual(self.outcome(None, tool='codegraph', op='query', diag={'nodes': ['parse_header']}),
                         'unresolved')
        self.assertEqual(self.outcome(None, tool='codegraph', op='query', diag={'file': False}),
                         'extraction:not-indexed')

    def test_unscored_cases(self):
        self.assertEqual(run.classify(case(score=False), 'codecite', 'search', 'ok', 1, None, []), 'unscored')


class ScheduleTest(unittest.TestCase):
    def test_pairs_alternate_order_and_every_call_runs_once_per_round(self):
        pairs = [('c1', 'g1'), ('c2', 'g2'), ('c3', None)]
        rounds = run.schedule(pairs, 6, seed=1)
        self.assertEqual(len(rounds), 6)
        for i, calls in enumerate(rounds):
            self.assertEqual(sorted(calls), sorted(['c1', 'g1', 'c2', 'g2', 'c3']))
            first = 'c' if i % 2 == 0 else 'g'
            for a, b in (('c1', 'g1'), ('c2', 'g2')):
                ia, ib = calls.index(a), calls.index(b)
                self.assertEqual(abs(ia - ib), 1)
                self.assertEqual(calls[min(ia, ib)][0], first)


class MetricsTest(unittest.TestCase):
    def test_hit_and_mrr(self):
        m = run.rank_metrics([1, 3, None, 11])
        self.assertEqual(m['n'], 4)
        self.assertAlmostEqual(m['hit@1'], 0.25)
        self.assertAlmostEqual(m['hit@10'], 0.5)
        self.assertAlmostEqual(m['mrr@10'], (1 + 1 / 3) / 4)

    def test_percentiles(self):
        self.assertEqual(run.percentile([5, 1, 3], 50), 3)
        self.assertEqual(run.percentile(list(range(1, 101)), 95), 95)


if __name__ == '__main__':
    unittest.main()
