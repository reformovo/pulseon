use crate::engine::EngineError;
use crate::engine::client::NativeClient;
use crate::model::comparison::{
    ComparisonOutcome, ComparisonPreference, ComparisonResult, EvidenceCompleteness,
    ObjectiveEvidence, ObjectiveMetric,
};
use crate::model::run::RunId;

impl NativeClient {
    /// Compares two Runs using their last effective objective values.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::DuplicateRunIdentity`] when both request roles
    /// use the same Run or a lookup/storage error when evidence cannot be read.
    pub fn compare_runs(
        &self,
        candidate_run_id: &RunId,
        reference_run_id: &RunId,
        objective: &ObjectiveMetric,
    ) -> Result<ComparisonResult, EngineError> {
        if candidate_run_id == reference_run_id {
            return Err(EngineError::DuplicateRunIdentity {
                run_id: candidate_run_id.as_str().to_owned(),
            });
        }
        let candidate = self.objective_evidence(candidate_run_id, objective)?;
        let reference = self.objective_evidence(reference_run_id, objective)?;
        Ok(compare_evidence(objective, candidate, reference))
    }
}

fn compare_evidence(
    objective: &ObjectiveMetric,
    candidate: ObjectiveEvidence,
    reference: ObjectiveEvidence,
) -> ComparisonResult {
    let completeness = candidate.completeness.max(reference.completeness);
    let numeric =
        candidate
            .usable_value()
            .zip(reference.usable_value())
            .map(|(candidate, reference)| {
                let raw_delta = candidate - reference;
                let relative_delta = (reference != 0.0).then_some(raw_delta / reference.abs());
                let normalized = objective.direction.normalize(raw_delta);
                let outcome = ComparisonOutcome::from_normalized_improvement(normalized);
                (raw_delta, relative_delta, normalized, outcome)
            });
    let (raw_delta, relative_delta, normalized_improvement, outcome) = match numeric {
        Some((raw, relative, normalized, outcome)) => {
            (Some(raw), relative, Some(normalized), Some(outcome))
        }
        None => (None, None, None, None),
    };
    let preference = match (completeness, outcome) {
        (EvidenceCompleteness::Complete, Some(ComparisonOutcome::Improved)) => {
            ComparisonPreference::Candidate
        }
        (EvidenceCompleteness::Complete, Some(ComparisonOutcome::Regressed)) => {
            ComparisonPreference::Reference
        }
        (EvidenceCompleteness::Complete, Some(ComparisonOutcome::Equal)) => {
            ComparisonPreference::NoPreference
        }
        _ => ComparisonPreference::Inconclusive,
    };
    ComparisonResult {
        objective: objective.clone(),
        candidate,
        reference,
        completeness,
        raw_delta,
        relative_delta,
        normalized_improvement,
        outcome,
        preference,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::comparison::{EvidenceReason, ObjectiveDirection};
    use crate::model::metric::{MetricKey, Step};
    use crate::model::run::RunStatus;

    use super::*;

    fn evidence(run_id: &str, value: f64, completeness: EvidenceCompleteness) -> ObjectiveEvidence {
        ObjectiveEvidence {
            run_id: RunId::from_string(run_id),
            run_status: RunStatus::Finished,
            last_step: Some(Step::new(4)),
            last_value_f64: Some(value),
            completeness,
            reasons: Vec::new(),
        }
    }

    fn objective(direction: ObjectiveDirection) -> ObjectiveMetric {
        ObjectiveMetric {
            metric_key: MetricKey::from_string("loss"),
            direction,
        }
    }

    #[test]
    fn minimize_comparison_normalizes_delta_and_prefers_candidate() {
        let result = compare_evidence(
            &objective(ObjectiveDirection::Minimize),
            evidence("candidate", 2.0, EvidenceCompleteness::Complete),
            evidence("reference", 4.0, EvidenceCompleteness::Complete),
        );

        assert_eq!(result.raw_delta, Some(-2.0));
        assert_eq!(result.relative_delta, Some(-0.5));
        assert_eq!(result.normalized_improvement, Some(2.0));
        assert_eq!(result.outcome, Some(ComparisonOutcome::Improved));
        assert_eq!(result.preference, ComparisonPreference::Candidate);
    }

    #[test]
    fn zero_reference_omits_relative_delta_but_keeps_outcome() {
        let result = compare_evidence(
            &objective(ObjectiveDirection::Maximize),
            evidence("candidate", 1.0, EvidenceCompleteness::Complete),
            evidence("reference", 0.0, EvidenceCompleteness::Complete),
        );

        assert_eq!(result.relative_delta, None);
        assert_eq!(result.outcome, Some(ComparisonOutcome::Improved));
    }

    #[test]
    fn partial_numeric_evidence_keeps_outcome_but_is_inconclusive() {
        let mut candidate = evidence("candidate", 1.0, EvidenceCompleteness::Partial);
        candidate.reasons.push(EvidenceReason::RunRunning);
        let result = compare_evidence(
            &objective(ObjectiveDirection::Minimize),
            candidate,
            evidence("reference", 2.0, EvidenceCompleteness::Complete),
        );

        assert_eq!(result.outcome, Some(ComparisonOutcome::Improved));
        assert_eq!(result.preference, ComparisonPreference::Inconclusive);
    }
}
